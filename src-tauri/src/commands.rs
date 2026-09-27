//! Tauri command surface. Thin wrappers over `satchel-core`; all errors are
//! mapped to strings at this boundary. The frontend only ever talks to these.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::Ordering;

use satchel_core::canvas::{self, Canvas};
use satchel_core::cloud::{self, Provider, SyncLocation};
use satchel_core::embed::Embedder;
use satchel_core::index::{self, IndexStats};
use satchel_core::query::{self, QueryResult};
use satchel_core::search::{self, semantic, SearchHit};
use satchel_core::{agents, db, paths, rusqlite, vault, watch};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::state::{AppState, Vault};

fn es<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// Move a file or folder to the system Trash. On macOS this goes through
/// NSFileManager rather than scripting Finder, so there's no Automation prompt.
fn move_to_trash(path: &Path) -> Result<(), String> {
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut ctx = trash::TrashContext::default();
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        ctx.set_delete_method(DeleteMethod::NsFileManager);
    }
    ctx.delete(path).map_err(|e| format!("Couldn't move to Trash: {e}"))
}

#[derive(Serialize)]
pub struct VaultInfo {
    pub root: String,
    pub note_count: i64,
    /// Which service syncs the vault folder (or `local`).
    pub provider: Provider,
    /// Human name for `provider`, e.g. "iCloud Drive".
    pub provider_name: String,
}

#[derive(Serialize)]
pub struct NoteMeta {
    pub id: i64,
    pub rel_path: String,
    pub title: String,
    pub mtime: i64,
}

#[derive(Serialize)]
pub struct NoteContent {
    pub rel_path: String,
    pub content: String,
}

#[derive(Serialize)]
pub struct Backlink {
    pub rel_path: String,
    pub title: String,
    pub unlinked: bool,
}

#[derive(Serialize)]
pub struct GraphNode {
    pub id: String,
    pub title: String,
}
#[derive(Serialize)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
}
#[derive(Serialize)]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

#[derive(Serialize)]
pub struct TagCount {
    pub tag: String,
    pub count: i64,
}

/// Progress of the background semantic indexer (event `embed-status`).
#[derive(Clone, Serialize)]
pub struct EmbedStatus {
    pub running: bool,
    pub done: usize,
    pub total: usize,
    pub error: Option<String>,
}

// ---- helpers ---------------------------------------------------------------

fn with_vault<T>(
    state: &State<AppState>,
    f: impl FnOnce(&mut Vault) -> Result<T, String>,
) -> Result<T, String> {
    let mut guard = state.vault.lock().map_err(|_| "state poisoned")?;
    let vault = guard.as_mut().ok_or("no vault is open")?;
    f(vault)
}

/// Load the embedder (if needed) and make sure the vector table exists, so
/// semantic search works — returning partial results while indexing runs.
fn ensure_semantic_ready(vault: &mut Vault, model_dir: &Path) -> Result<(), String> {
    if vault.embedder.is_none() {
        vault.embedder = Some(Embedder::from_dir(model_dir).map_err(es)?);
    }
    let dim = vault.embedder.as_ref().map(|e| e.dim()).unwrap_or(0) as u32;
    semantic::ensure_semantic(&vault.conn, dim).map_err(es)
}

/// Reject paths that resolve to the vault root itself ("", ".", "/"), so
/// folder and delete operations can never act on the whole vault.
fn require_subpath(rel: &str) -> Result<(), String> {
    let has_part = Path::new(rel.trim_matches('/'))
        .components()
        .any(|c| matches!(c, Component::Normal(_)));
    if has_part {
        Ok(())
    } else {
        Err("refusing to operate on the vault root".into())
    }
}

fn is_case_only_rename(from: &str, to: &str) -> bool {
    from != to && from.to_lowercase() == to.to_lowercase()
}

fn note_count(conn: &rusqlite::Connection) -> Result<i64, String> {
    conn.query_row("SELECT count(*) FROM notes", [], |r| r.get(0))
        .map_err(es)
}

fn vault_info(root: &Path, conn: &rusqlite::Connection) -> Result<VaultInfo, String> {
    let provider = cloud::provider_for_path(root);
    Ok(VaultInfo {
        root: root.to_string_lossy().into_owned(),
        note_count: note_count(conn)?,
        provider,
        provider_name: provider.display_name().to_string(),
    })
}

fn last_vault_file(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|d| d.join("last_vault.txt"))
}

fn remember_last_vault(app: &AppHandle, root: &Path) {
    if let Some(file) = last_vault_file(app) {
        if let Some(parent) = file.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(file, root.to_string_lossy().as_bytes());
    }
}

/// Watch for external edits (AI tools, other editors, cloud sync): reconcile
/// the index and notify the frontend. Own-writes are idempotent re-indexes.
fn start_watcher(app: &AppHandle, root: &Path) -> Option<watch::WatchGuard> {
    let app = app.clone();
    watch::start(root, move |events| {
        if let Some(state) = app.try_state::<AppState>() {
            if let Ok(mut guard) = state.vault.lock() {
                if let Some(v) = guard.as_mut() {
                    for ev in &events {
                        let _ = index::index_single(&v.conn, &v.root, &ev.rel_path);
                        if let (false, Some(emb)) = (ev.removed, v.embedder.as_ref()) {
                            if let Ok(Some(id)) = index::note_id_by_path(&v.conn, &ev.rel_path) {
                                let _ = semantic::index_note_embeddings(&v.conn, emb, id);
                            }
                        }
                    }
                }
            }
        }
        let _ = app.emit("vault-changed", events);
    })
    .ok()
}

fn open_vault_at(
    app: &AppHandle,
    state: &State<AppState>,
    root: PathBuf,
) -> Result<VaultInfo, String> {
    if !root.is_dir() {
        return Err(format!("not a folder: {}", root.display()));
    }
    // The index is per-machine, outside the (usually cloud-synced) vault.
    let db_path = paths::index_db_path(&root);
    let conn = db::open(&db_path).map_err(es)?;
    index::reindex_all(&conn, &root).map_err(es)?;
    let query_conn = query::open_query_connection(&db_path).map_err(es)?;
    let info = vault_info(&root, &conn)?;
    let watch = start_watcher(app, &root);

    *state.vault.lock().map_err(|_| "state poisoned")? = Some(Vault {
        root: root.clone(),
        conn,
        query_conn,
        embedder: None,
        watch,
    });
    remember_last_vault(app, &root);
    spawn_semantic_indexer(app.clone());
    Ok(info)
}

fn welcome_note(provider: Provider) -> String {
    let sync = match provider {
        Provider::Local => "They're stored only on this computer.".to_string(),
        p => format!(
            "{} keeps them in sync across your devices — Satchel itself never uploads anything.",
            p.display_name()
        ),
    };
    format!(
        "# Welcome to Satchel\n\n\
         Your notes are plain Markdown files in this folder. {sync}\n\n\
         - Create a note with **+**, or a folder (notebook) with the folder button next to it.\n\
         - Link notes with `[[double brackets]]`.\n\
         - Paste or drop images straight into a note.\n\
         - Press **⌘K** (Ctrl+K) to search by words or by meaning, and **⌘⇧P** (Ctrl+Shift+P) for every command.\n\n\
         Delete this note whenever you like.\n"
    )
}

// ---- vault lifecycle -------------------------------------------------------

/// Cloud-synced folders (iCloud Drive, Google Drive, …) plus a local option.
#[tauri::command(async)]
pub fn detect_sync_locations() -> Vec<SyncLocation> {
    cloud::detect_locations()
}

/// Create (if needed) and open a vault folder, adding a welcome note to a new one.
#[tauri::command(async)]
pub fn create_vault(
    app: AppHandle,
    state: State<AppState>,
    path: String,
) -> Result<VaultInfo, String> {
    let root = PathBuf::from(&path);
    fs::create_dir_all(&root).map_err(|e| format!("Couldn't create {path}: {e}"))?;
    if vault::scan_vault(&root).is_empty() {
        let welcome = welcome_note(cloud::provider_for_path(&root));
        vault::write_note_atomic(&root.join("Welcome to Satchel.md"), &welcome).map_err(es)?;
    }
    open_vault_at(&app, &state, root)
}

#[tauri::command(async)]
pub fn open_vault(
    app: AppHandle,
    state: State<AppState>,
    path: String,
) -> Result<VaultInfo, String> {
    open_vault_at(&app, &state, PathBuf::from(path))
}

#[tauri::command(async)]
pub fn current_vault(state: State<AppState>) -> Result<Option<VaultInfo>, String> {
    let guard = state.vault.lock().map_err(|_| "state poisoned")?;
    match guard.as_ref() {
        Some(v) => Ok(Some(vault_info(&v.root, &v.conn)?)),
        None => Ok(None),
    }
}

#[tauri::command(async)]
pub fn close_vault(state: State<AppState>) -> Result<(), String> {
    *state.vault.lock().map_err(|_| "state poisoned")? = None;
    Ok(())
}

#[tauri::command(async)]
pub fn get_last_vault(app: AppHandle) -> Option<String> {
    last_vault_file(&app)
        .and_then(|f| fs::read_to_string(f).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[tauri::command(async)]
pub fn reindex(state: State<AppState>) -> Result<IndexStats, String> {
    with_vault(&state, |v| {
        let root = v.root.clone();
        index::reindex_all(&v.conn, &root).map_err(es)
    })
}

// ---- semantic indexing -----------------------------------------------------

/// Start the background semantic indexer (no-op if already running). It
/// embeds one note at a time, releasing the vault lock between notes, so the
/// UI stays responsive on large vaults. Progress is emitted as `embed-status`.
pub fn spawn_semantic_indexer(app: AppHandle) {
    if app.state::<AppState>().embedding.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let result = run_semantic_indexer(&app);
        app.state::<AppState>().embedding.store(false, Ordering::SeqCst);
        let status = match result {
            Ok((done, total)) => EmbedStatus { running: false, done, total, error: None },
            Err(e) => EmbedStatus { running: false, done: 0, total: 0, error: Some(e) },
        };
        let _ = app.emit("embed-status", status);
    });
}

fn run_semantic_indexer(app: &AppHandle) -> Result<(usize, usize), String> {
    let state = app.state::<AppState>();
    let (root, needs_model) = {
        let guard = state.vault.lock().map_err(|_| "state poisoned")?;
        let v = guard.as_ref().ok_or("no vault is open")?;
        (v.root.clone(), v.embedder.is_none())
    };
    if needs_model {
        // Load the model without holding the lock.
        let embedder = Embedder::from_dir(&state.model_dir).map_err(es)?;
        let mut guard = state.vault.lock().map_err(|_| "state poisoned")?;
        if let Some(v) = guard.as_mut().filter(|v| v.root == root) {
            if v.embedder.is_none() {
                v.embedder = Some(embedder);
            }
        }
    }
    let pending = {
        let mut guard = state.vault.lock().map_err(|_| "state poisoned")?;
        let Some(v) = guard.as_mut().filter(|v| v.root == root) else {
            return Ok((0, 0));
        };
        ensure_semantic_ready(v, &state.model_dir)?;
        semantic::pending_notes(&v.conn).map_err(es)?
    };
    let total = pending.len();
    let _ = app.emit(
        "embed-status",
        EmbedStatus { running: true, done: 0, total, error: None },
    );
    for (i, id) in pending.into_iter().enumerate() {
        {
            let guard = state.vault.lock().map_err(|_| "state poisoned")?;
            let Some(v) = guard.as_ref().filter(|v| v.root == root) else {
                return Ok((i, total)); // vault switched or closed
            };
            if let Some(embedder) = v.embedder.as_ref() {
                let _ = semantic::index_note_embeddings(&v.conn, embedder, id);
            }
        }
        let done = i + 1;
        if done % 10 == 0 || done == total {
            let _ = app.emit(
                "embed-status",
                EmbedStatus { running: true, done, total, error: None },
            );
        }
    }
    Ok((total, total))
}

/// Rebuild all embeddings from scratch (in the background).
#[tauri::command(async)]
pub fn embed_vault(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    if state.embedding.load(Ordering::SeqCst) {
        return Err("Semantic indexing is already running".into());
    }
    with_vault(&state, |v| {
        let _ = v.conn.execute("DELETE FROM vec_chunks", []); // may not exist yet
        v.conn.execute("DELETE FROM chunks", []).map_err(es)?;
        Ok(())
    })?;
    spawn_semantic_indexer(app);
    Ok(())
}

/// Opt-in: add AGENTS.md / CLAUDE.md so AI coding tools understand the vault.
#[tauri::command(async)]
pub fn add_agent_docs(state: State<AppState>) -> Result<(), String> {
    with_vault(&state, |v| {
        agents::ensure_agent_docs(&v.root).map_err(es)?;
        for doc in ["AGENTS.md", "CLAUDE.md"] {
            index::index_single(&v.conn, &v.root, doc).map_err(es)?;
        }
        Ok(())
    })
}

/// Ready-to-paste MCP setup for AI tools. The installed app doubles as the MCP
/// server (`<app> mcp --vault <folder>`), so no separate CLI is needed.
#[derive(Serialize)]
pub struct AgentSetup {
    pub claude_code: String,
    pub codex: String,
}

#[tauri::command(async)]
pub fn agent_setup(state: State<AppState>) -> Result<AgentSetup, String> {
    let root = with_vault(&state, |v| Ok(v.root.clone()))?;
    let exe = std::env::current_exe().map_err(es)?;
    let (exe, root) = (exe.to_string_lossy(), root.to_string_lossy());
    // JSON string literals are valid TOML basic strings (escapes included).
    let q = |s: &str| serde_json::to_string(s).unwrap_or_default();
    Ok(AgentSetup {
        claude_code: format!(
            "claude mcp add --transport stdio satchel -- {} mcp --vault {}",
            q(&exe),
            q(&root)
        ),
        codex: format!(
            "[mcp_servers.satchel]\ncommand = {}\nargs = [\"mcp\", \"--vault\", {}]\n",
            q(&exe),
            q(&root)
        ),
    })
}

// ---- notes -----------------------------------------------------------------

#[tauri::command(async)]
pub fn list_notes(state: State<AppState>) -> Result<Vec<NoteMeta>, String> {
    with_vault(&state, |v| {
        let mut stmt = v
            .conn
            .prepare("SELECT id, rel_path, title, mtime FROM notes ORDER BY rel_path")
            .map_err(es)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(NoteMeta {
                    id: r.get(0)?,
                    rel_path: r.get(1)?,
                    title: r.get(2)?,
                    mtime: r.get(3)?,
                })
            })
            .map_err(es)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(es)
    })
}

#[tauri::command(async)]
pub fn read_note(state: State<AppState>, rel_path: String) -> Result<NoteContent, String> {
    let root = with_vault(&state, |v| Ok(v.root.clone()))?;
    let abs = vault::safe_join(&root, &rel_path).map_err(es)?;
    // A cloud-only note is downloaded by the OS when read; do that without
    // holding the vault lock, then index it now that its content is local.
    let was_cloud_only = fs::metadata(&abs)
        .map(|m| vault::is_dataless(&m))
        .unwrap_or(false);
    let content = vault::read_note(&abs).map_err(es)?;
    if was_cloud_only {
        with_vault(&state, |v| {
            if v.root == root {
                index::index_single(&v.conn, &v.root, &rel_path).map_err(es)?;
            }
            Ok(())
        })?;
    }
    Ok(NoteContent { rel_path, content })
}

fn reindex_one(v: &mut Vault, rel_path: &str) -> Result<(), String> {
    let root = v.root.clone();
    let id = index::index_single(&v.conn, &root, rel_path).map_err(es)?;
    // Keep embeddings current if they've been built this session.
    if let (Some(embedder), Some(id)) = (v.embedder.as_ref(), id) {
        semantic::index_note_embeddings(&v.conn, embedder, id).map_err(es)?;
    }
    Ok(())
}

#[tauri::command(async)]
pub fn write_note(state: State<AppState>, rel_path: String, content: String) -> Result<(), String> {
    with_vault(&state, |v| {
        let abs = vault::safe_join(&v.root, &rel_path).map_err(es)?;
        vault::write_note_atomic(&abs, &content).map_err(es)?;
        reindex_one(v, &rel_path)
    })
}

#[tauri::command(async)]
pub fn create_note(
    state: State<AppState>,
    rel_path: String,
    content: Option<String>,
) -> Result<String, String> {
    with_vault(&state, |v| {
        // Reject traversal/absolute up front; derived names below stay safe.
        vault::safe_join(&v.root, &rel_path).map_err(es)?;
        // Ensure a unique path: "Untitled.md" -> "Untitled 1.md" ...
        let (stem, ext) = match rel_path.rsplit_once('.') {
            Some((s, e)) => (s.to_string(), format!(".{e}")),
            None => (rel_path.clone(), String::new()),
        };
        let mut final_rel = rel_path.clone();
        let mut n = 1;
        while v.root.join(&final_rel).exists() {
            final_rel = format!("{stem} {n}{ext}");
            n += 1;
        }
        let title = final_rel
            .rsplit('/')
            .next()
            .and_then(|f| f.strip_suffix(".md"))
            .unwrap_or(&final_rel);
        let body = content.unwrap_or_else(|| format!("# {title}\n\n"));
        let abs = vault::safe_join(&v.root, &final_rel).map_err(es)?;
        vault::write_note_atomic(&abs, &body).map_err(es)?;
        reindex_one(v, &final_rel)?;
        Ok(final_rel)
    })
}

#[tauri::command(async)]
pub fn rename_note(state: State<AppState>, from: String, to: String) -> Result<(), String> {
    require_subpath(&from)?;
    require_subpath(&to)?;
    with_vault(&state, |v| {
        let abs_from = vault::safe_join(&v.root, &from).map_err(es)?;
        let abs_to = vault::safe_join(&v.root, &to).map_err(es)?;
        if !abs_from.is_file() {
            return Err(format!("\"{from}\" is not a note"));
        }
        // A case-only rename finds "itself" on case-insensitive disks.
        if abs_to.exists() && !is_case_only_rename(&from, &to) {
            return Err(format!("\"{to}\" already exists"));
        }
        if let Some(parent) = abs_to.parent() {
            fs::create_dir_all(parent).map_err(es)?;
        }
        fs::rename(&abs_from, &abs_to).map_err(es)?;
        index::remove_note(&v.conn, &from).map_err(es)?;
        reindex_one(v, &to)
    })
}

/// Move a note to the system Trash (recoverable), then drop it from the index.
#[tauri::command(async)]
pub fn delete_note(state: State<AppState>, rel_path: String) -> Result<(), String> {
    require_subpath(&rel_path)?;
    with_vault(&state, |v| {
        let abs = vault::safe_join(&v.root, &rel_path).map_err(es)?;
        if abs.is_dir() {
            return Err("not a note".into());
        }
        if abs.is_file() {
            move_to_trash(&abs)?;
        }
        reindex_one(v, &rel_path)
    })
}

// ---- folders (notebooks) ---------------------------------------------------

#[tauri::command(async)]
pub fn list_folders(state: State<AppState>) -> Result<Vec<String>, String> {
    with_vault(&state, |v| Ok(vault::list_dirs(&v.root)))
}

#[tauri::command(async)]
pub fn create_folder(state: State<AppState>, rel_path: String) -> Result<(), String> {
    require_subpath(&rel_path)?;
    with_vault(&state, |v| {
        let abs = vault::safe_join(&v.root, &rel_path).map_err(es)?;
        if abs.exists() {
            return Err(format!("\"{rel_path}\" already exists"));
        }
        fs::create_dir_all(&abs).map_err(es)
    })
}

/// Rename or move a folder; its notes are re-indexed under their new paths.
#[tauri::command(async)]
pub fn rename_folder(
    app: AppHandle,
    state: State<AppState>,
    from: String,
    to: String,
) -> Result<(), String> {
    require_subpath(&from)?;
    require_subpath(&to)?;
    with_vault(&state, |v| {
        let abs_from = vault::safe_join(&v.root, &from).map_err(es)?;
        let abs_to = vault::safe_join(&v.root, &to).map_err(es)?;
        if !abs_from.is_dir() {
            return Err(format!("\"{from}\" is not a folder"));
        }
        if abs_to.exists() && !is_case_only_rename(&from, &to) {
            return Err(format!("\"{to}\" already exists"));
        }
        if abs_to.starts_with(&abs_from) {
            return Err("can't move a folder into itself".into());
        }
        if let Some(parent) = abs_to.parent() {
            fs::create_dir_all(parent).map_err(es)?;
        }
        fs::rename(&abs_from, &abs_to).map_err(es)?;
        let root = v.root.clone();
        index::reindex_all(&v.conn, &root).map_err(es)?;
        Ok(())
    })?;
    spawn_semantic_indexer(app); // moved notes get fresh embeddings
    Ok(())
}

/// Move a folder (and everything in it) to the system Trash.
#[tauri::command(async)]
pub fn delete_folder(state: State<AppState>, rel_path: String) -> Result<(), String> {
    require_subpath(&rel_path)?;
    with_vault(&state, |v| {
        let abs = vault::safe_join(&v.root, &rel_path).map_err(es)?;
        if !abs.is_dir() {
            return Err(format!("\"{rel_path}\" is not a folder"));
        }
        move_to_trash(&abs)?;
        let root = v.root.clone();
        index::reindex_all(&v.conn, &root).map_err(es)?;
        Ok(())
    })
}

// ---- attachments -----------------------------------------------------------

#[tauri::command(async)]
pub fn save_attachment(
    state: State<AppState>,
    bytes: Vec<u8>,
    ext: String,
) -> Result<String, String> {
    with_vault(&state, |v| {
        let safe_ext: String = ext.chars().filter(|c| c.is_alphanumeric()).take(8).collect();
        let safe_ext = if safe_ext.is_empty() {
            "bin".to_string()
        } else {
            safe_ext.to_lowercase()
        };
        let hash = vault::content_hash(&bytes);
        let rel = format!("attachments/{hash}.{safe_ext}");
        let abs = vault::safe_join(&v.root, &rel).map_err(es)?;
        if !abs.exists() {
            vault::write_bytes_atomic(&abs, &bytes).map_err(es)?;
        }
        Ok(rel)
    })
}

#[tauri::command(async)]
pub fn read_attachment(state: State<AppState>, rel_path: String) -> Result<Vec<u8>, String> {
    with_vault(&state, |v| {
        let abs = vault::safe_join(&v.root, &rel_path).map_err(es)?;
        fs::read(&abs).map_err(es)
    })
}

// ---- canvas (JSONCanvas) ---------------------------------------------------

#[tauri::command(async)]
pub fn list_canvases(state: State<AppState>) -> Result<Vec<String>, String> {
    with_vault(&state, |v| Ok(vault::list_by_ext(&v.root, "canvas")))
}

#[tauri::command(async)]
pub fn read_canvas(state: State<AppState>, rel_path: String) -> Result<Canvas, String> {
    with_vault(&state, |v| {
        let abs = vault::safe_join(&v.root, &rel_path).map_err(es)?;
        let json = if abs.exists() {
            vault::read_note(&abs).map_err(es)?
        } else {
            String::new()
        };
        canvas::parse_canvas(&json).map_err(es)
    })
}

#[tauri::command(async)]
pub fn write_canvas(
    state: State<AppState>,
    rel_path: String,
    canvas: Canvas,
) -> Result<(), String> {
    with_vault(&state, |v| {
        let abs = vault::safe_join(&v.root, &rel_path).map_err(es)?;
        let json = canvas::serialize_canvas(&canvas).map_err(es)?;
        vault::write_note_atomic(&abs, &json).map_err(es)
    })
}

#[tauri::command(async)]
pub fn create_canvas(state: State<AppState>, rel_path: String) -> Result<String, String> {
    with_vault(&state, |v| {
        let mut base = rel_path;
        if !base.to_lowercase().ends_with(".canvas") {
            base = format!("{base}.canvas");
        }
        let stem = base.trim_end_matches(".canvas").to_string();
        let mut final_rel = base.clone();
        let mut n = 1;
        while v.root.join(&final_rel).exists() {
            final_rel = format!("{stem} {n}.canvas");
            n += 1;
        }
        let abs = vault::safe_join(&v.root, &final_rel).map_err(es)?;
        let json = canvas::serialize_canvas(&Canvas::default()).map_err(es)?;
        vault::write_note_atomic(&abs, &json).map_err(es)?;
        Ok(final_rel)
    })
}

// ---- search & query --------------------------------------------------------

#[tauri::command(async)]
pub fn search_fulltext(
    state: State<AppState>,
    query: String,
    limit: usize,
) -> Result<Vec<SearchHit>, String> {
    with_vault(&state, |v| {
        search::fts::search_fulltext(&v.conn, &query, limit).map_err(es)
    })
}

#[tauri::command(async)]
pub fn search_semantic(
    state: State<AppState>,
    query: String,
    limit: usize,
) -> Result<Vec<SearchHit>, String> {
    let model_dir = state.model_dir.clone();
    with_vault(&state, |v| {
        ensure_semantic_ready(v, &model_dir)?;
        let embedder = v.embedder.as_ref().unwrap();
        semantic::search_semantic(&v.conn, embedder, &query, limit).map_err(es)
    })
}

#[tauri::command(async)]
pub fn search_hybrid(
    state: State<AppState>,
    query: String,
    limit: usize,
) -> Result<Vec<SearchHit>, String> {
    let model_dir = state.model_dir.clone();
    with_vault(&state, |v| {
        ensure_semantic_ready(v, &model_dir)?;
        let embedder = v.embedder.as_ref().unwrap();
        semantic::search_hybrid(&v.conn, embedder, &query, limit).map_err(es)
    })
}

#[tauri::command(async)]
pub fn run_query(
    state: State<AppState>,
    sql: String,
    max_rows: usize,
) -> Result<QueryResult, String> {
    with_vault(&state, |v| {
        query::run_query(&v.query_conn, &sql, max_rows).map_err(es)
    })
}

// ---- links, graph, tags ----------------------------------------------------

#[tauri::command(async)]
pub fn get_backlinks(state: State<AppState>, rel_path: String) -> Result<Vec<Backlink>, String> {
    with_vault(&state, |v| {
        let Some(note_id) = index::note_id_by_path(&v.conn, &rel_path).map_err(es)? else {
            return Ok(Vec::new());
        };
        // Resolved incoming links.
        let mut linked: Vec<Backlink> = {
            let mut stmt = v
                .conn
                .prepare(
                    "SELECT DISTINCT n.rel_path, n.title FROM links l \
                     JOIN notes n ON n.id = l.src_note_id \
                     WHERE l.target_note_id = ?1 AND l.src_note_id != ?1 ORDER BY n.rel_path",
                )
                .map_err(es)?;
            let rows = stmt
                .query_map([note_id], |r| {
                    Ok(Backlink {
                        rel_path: r.get(0)?,
                        title: r.get(1)?,
                        unlinked: false,
                    })
                })
                .map_err(es)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(es)?
        };

        // Unlinked mentions: notes whose text matches the title but don't link here.
        let title: String = v
            .conn
            .query_row("SELECT title FROM notes WHERE id = ?1", [note_id], |r| r.get(0))
            .map_err(es)?;
        if title.len() >= 3 {
            let already: std::collections::HashSet<String> =
                linked.iter().map(|b| b.rel_path.clone()).collect();
            let hits = search::fts::search_fulltext(&v.conn, &title, 50).map_err(es)?;
            for h in hits {
                if h.rel_path != rel_path && !already.contains(&h.rel_path) {
                    linked.push(Backlink {
                        rel_path: h.rel_path,
                        title: h.title,
                        unlinked: true,
                    });
                }
            }
        }
        Ok(linked)
    })
}

#[tauri::command(async)]
pub fn get_graph(state: State<AppState>) -> Result<GraphData, String> {
    with_vault(&state, |v| {
        let nodes = {
            let mut stmt = v
                .conn
                .prepare("SELECT rel_path, title FROM notes")
                .map_err(es)?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(GraphNode {
                        id: r.get(0)?,
                        title: r.get(1)?,
                    })
                })
                .map_err(es)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(es)?
        };
        let edges = {
            let mut stmt = v
                .conn
                .prepare(
                    "SELECT s.rel_path, t.rel_path FROM links l \
                     JOIN notes s ON s.id = l.src_note_id \
                     JOIN notes t ON t.id = l.target_note_id WHERE l.resolved = 1",
                )
                .map_err(es)?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(GraphEdge {
                        source: r.get(0)?,
                        target: r.get(1)?,
                    })
                })
                .map_err(es)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(es)?
        };
        Ok(GraphData { nodes, edges })
    })
}

#[tauri::command(async)]
pub fn get_tags(state: State<AppState>) -> Result<Vec<TagCount>, String> {
    with_vault(&state, |v| {
        let mut stmt = v
            .conn
            .prepare("SELECT tag, count(*) c FROM tags GROUP BY tag ORDER BY c DESC, tag")
            .map_err(es)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(TagCount {
                    tag: r.get(0)?,
                    count: r.get(1)?,
                })
            })
            .map_err(es)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(es)
    })
}

#[tauri::command(async)]
pub fn notes_for_tag(state: State<AppState>, tag: String) -> Result<Vec<String>, String> {
    with_vault(&state, |v| {
        let mut stmt = v
            .conn
            .prepare(
                "SELECT n.rel_path FROM tags t JOIN notes n ON n.id = t.note_id \
                 WHERE t.tag = ?1 ORDER BY n.rel_path",
            )
            .map_err(es)?;
        let rows = stmt
            .query_map([tag], |r| r.get::<_, String>(0))
            .map_err(es)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(es)
    })
}
