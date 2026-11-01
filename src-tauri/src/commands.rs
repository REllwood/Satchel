//! Tauri command surface. Thin wrappers over `satchel-core`; all errors are
//! mapped to strings at this boundary. The frontend only ever talks to these.

use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use satchel_core::canvas::{self, Canvas};
use satchel_core::cloud::{self, Provider, SyncLocation};
use satchel_core::embed::EMBEDDING_DIM;
use satchel_core::index::{self, IndexStats};
use satchel_core::query::{self, QueryResult};
use satchel_core::search::{self, semantic, SearchHit};
use satchel_core::watch::VaultEvent;
use satchel_core::{agents, db, paths, rusqlite, vault, watch};
use serde::Serialize;
use tauri::ipc::{InvokeBody, Request, Response};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

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
    /// The welcome note, when `create_vault` just wrote one into a new vault.
    pub welcome_note: Option<String>,
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

/// True if a rename's target already exists as a different file or folder. On
/// a case-insensitive disk, renaming `notes.md` to `Notes.md` finds the note
/// itself at the target, which is fine. On a case-sensitive disk the two can
/// be different files, and the rename must not replace the other one.
fn target_taken(from: &Path, to: &Path) -> bool {
    to.exists() && !same_file::is_same_file(from, to).unwrap_or(false)
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
        welcome_note: None,
    })
}

fn last_vault_file<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|d| d.join("last_vault.txt"))
}

fn remember_last_vault<R: Runtime>(app: &AppHandle<R>, root: &Path) {
    if let Some(file) = last_vault_file(app) {
        if let Some(parent) = file.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(file, root.to_string_lossy().as_bytes());
    }
}

/// Watch for external edits (AI tools, other editors, cloud sync): reconcile
/// the index and notify the frontend. Own-writes are idempotent re-indexes.
/// Changed notes are embedded afterwards by the background indexer, so a big
/// sync never holds the vault lock while the model runs. A folder change the
/// OS reports only as a folder triggers a full rescan, and the notes it moved
/// or removed are reported one by one, so an open note can follow its folder.
fn start_watcher<R: Runtime>(app: &AppHandle<R>, root: &Path) -> Option<watch::WatchGuard> {
    let app = app.clone();
    let watched = root.to_path_buf();
    watch::start(root, move |batch| {
        let mut events = batch.events;
        // The scan is the slow part, so it runs before taking the lock.
        let scan = batch.rescan.then(|| vault::scan_vault_report(&watched).ok()).flatten();
        let mut embed = false;
        if let Some(state) = app.try_state::<AppState>() {
            if let Ok(guard) = state.vault.lock() {
                // Drop a late batch from a vault that has since been closed.
                if let Some(v) = guard.as_ref().filter(|v| v.root == watched) {
                    let paths: Vec<&str> = events.iter().map(|e| e.rel_path.as_str()).collect();
                    let _ = index::index_paths(&v.conn, &v.root, &paths);
                    if let Some(scan) = &scan {
                        let before = note_paths(&v.conn);
                        let _ = index::apply_scan(&v.conn, scan);
                        let after = note_paths(&v.conn);
                        let mut seen: HashSet<String> =
                            events.iter().map(|e| e.rel_path.clone()).collect();
                        for rel in before.difference(&after) {
                            if seen.insert(rel.clone()) {
                                events.push(VaultEvent { rel_path: rel.clone(), removed: true });
                            }
                        }
                        for rel in after.difference(&before) {
                            if seen.insert(rel.clone()) {
                                events.push(VaultEvent { rel_path: rel.clone(), removed: false });
                            }
                        }
                    }
                    embed = state.loaded_embedder().is_some();
                }
            }
        }
        if embed {
            spawn_semantic_indexer(app.clone());
        }
        let _ = app.emit("vault-changed", events);
    })
    .ok()
}

fn note_paths(conn: &rusqlite::Connection) -> HashSet<String> {
    conn.prepare("SELECT rel_path FROM notes")
        .and_then(|mut stmt| {
            stmt.query_map([], |r| r.get::<_, String>(0))?
                .collect::<Result<HashSet<_>, _>>()
        })
        .unwrap_or_default()
}

fn open_vault_at<R: Runtime>(
    app: &AppHandle<R>,
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
    semantic::ensure_semantic(&conn, EMBEDDING_DIM as u32).map_err(es)?;
    let query_conn = query::open_query_connection(&db_path).map_err(es)?;
    let info = vault_info(&root, &conn)?;
    let watch = start_watcher(app, &root);

    *state.vault.lock().map_err(|_| "state poisoned")? = Some(Vault {
        root: root.clone(),
        conn,
        query_conn,
        watch,
    });
    remember_last_vault(app, &root);
    spawn_semantic_indexer(app.clone());
    Ok(info)
}

fn welcome_note(provider: Provider) -> String {
    let sync = match provider {
        Provider::Local => "They are stored only on this computer.".to_string(),
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
pub fn create_vault<R: Runtime>(
    app: AppHandle<R>,
    state: State<AppState>,
    path: String,
) -> Result<VaultInfo, String> {
    const WELCOME: &str = "Welcome to Satchel.md";
    let root = PathBuf::from(&path);
    fs::create_dir_all(&root).map_err(|e| format!("Could not create {path}: {e}"))?;
    let mut wrote_welcome = false;
    if vault::scan_vault(&root).is_empty() {
        let welcome = welcome_note(cloud::provider_for_path(&root));
        vault::write_note_atomic(&root.join(WELCOME), &welcome).map_err(es)?;
        wrote_welcome = true;
    }
    let mut info = open_vault_at(&app, &state, root)?;
    info.welcome_note = wrote_welcome.then(|| WELCOME.to_string());
    Ok(info)
}

#[tauri::command(async)]
pub fn open_vault<R: Runtime>(
    app: AppHandle<R>,
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
pub fn get_last_vault<R: Runtime>(app: AppHandle<R>) -> Option<String> {
    last_vault_file(&app)
        .and_then(|f| fs::read_to_string(f).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Rescan the whole notes folder. The scan (the slow part on a cloud drive)
/// runs without the vault lock.
#[tauri::command(async)]
pub fn reindex<R: Runtime>(app: AppHandle<R>, state: State<AppState>) -> Result<IndexStats, String> {
    let root = with_vault(&state, |v| Ok(v.root.clone()))?;
    let scan = vault::scan_vault_report(&root)
        .map_err(|e| format!("Could not read the notes folder: {e}"))?;
    let stats = with_vault(&state, |v| {
        if v.root != root {
            return Err("the notes folder changed while it was being scanned".into());
        }
        index::apply_scan(&v.conn, &scan).map_err(es)
    })?;
    refresh_embeddings(&app, &state);
    Ok(stats)
}

// ---- semantic indexing -----------------------------------------------------

/// Runs of the background indexer that finish sooner than this, such as
/// re-embedding the note that was just saved, never show progress.
const SHOW_PROGRESS_AFTER: Duration = Duration::from_millis(750);

/// Progress reporting for one run of the background indexer (event `embed-status`).
struct Progress {
    started: Instant,
    shown: bool,
}

impl Progress {
    fn report<R: Runtime>(&mut self, app: &AppHandle<R>, done: usize, total: usize) {
        let too_soon = self.started.elapsed() < SHOW_PROGRESS_AFTER;
        if too_soon || (self.shown && !done.is_multiple_of(10)) {
            return;
        }
        self.shown = true;
        let _ = app.emit(
            "embed-status",
            EmbedStatus { running: true, done, total, error: None },
        );
    }
}

/// Ask the background semantic indexer to embed every note that needs it. It
/// starts if idle, or else runs another pass when its current one finishes.
/// It embeds one note at a time and runs the model without holding the vault
/// lock, so saves, searches and the rest of the UI stay responsive.
pub fn spawn_semantic_indexer<R: Runtime>(app: AppHandle<R>) {
    if !app.state::<AppState>().embed_queue.request() {
        return;
    }
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let mut progress = Progress { started: Instant::now(), shown: false };
        let result = loop {
            state.embed_queue.begin_pass();
            match run_semantic_indexer(&app, &mut progress) {
                Err(e) => {
                    state.embed_queue.stop();
                    break Err(e);
                }
                Ok(counts) => {
                    if !state.embed_queue.end_pass() {
                        break Ok(counts);
                    }
                }
            }
        };
        let status = match result {
            Ok((done, total)) => EmbedStatus { running: false, done, total, error: None },
            Err(e) => EmbedStatus { running: false, done: 0, total: 0, error: Some(e) },
        };
        // A quick run never showed progress, so there is nothing to clear.
        if progress.shown || status.error.is_some() {
            let _ = app.emit("embed-status", status);
        }
    });
}

/// One pass: embed every note that has content but no embeddings yet.
fn run_semantic_indexer<R: Runtime>(
    app: &AppHandle<R>,
    progress: &mut Progress,
) -> Result<(usize, usize), String> {
    let state = app.state::<AppState>();
    let root = with_vault(&state, |v| Ok(v.root.clone()))?;
    // Loads the model on first use, without holding the vault lock.
    let embedder = state.embedder()?;
    let pending = {
        let guard = state.vault.lock().map_err(|_| "state poisoned")?;
        let Some(v) = guard.as_ref().filter(|v| v.root == root) else {
            return Ok((0, 0));
        };
        semantic::pending_notes(&v.conn).map_err(es)?
    };
    let total = pending.len();
    for (i, id) in pending.into_iter().enumerate() {
        // Read the note under the lock...
        let job = {
            let guard = state.vault.lock().map_err(|_| "state poisoned")?;
            let Some(v) = guard.as_ref().filter(|v| v.root == root) else {
                return Ok((i, total)); // vault switched or closed
            };
            semantic::prepare_note_embeddings(&v.conn, id)
        };
        // ...run the model without it...
        if let Ok(Some(job)) = job {
            if let Ok(vectors) = embedder.embed(&job.texts()) {
                // ...and keep the result only if the note did not change meanwhile.
                let guard = state.vault.lock().map_err(|_| "state poisoned")?;
                if let Some(v) = guard.as_ref().filter(|v| v.root == root) {
                    let _ = semantic::store_note_embeddings(&v.conn, &job, &vectors);
                }
            }
        }
        let done = i + 1;
        if done < total {
            progress.report(app, done, total);
        }
    }
    Ok((total, total))
}

/// Rebuild all embeddings from scratch (in the background).
#[tauri::command(async)]
pub fn embed_vault<R: Runtime>(app: AppHandle<R>, state: State<AppState>) -> Result<(), String> {
    if state.embed_queue.is_running() {
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
    /// Set when the app runs from a place that will not last, such as the
    /// disk image it was downloaded in.
    pub warning: Option<String>,
}

/// Quote `s` for the shell the user pastes the command into: single quotes
/// for zsh and bash (so `$`, backticks and spaces stay literal), and for
/// PowerShell on Windows.
fn shell_quote(s: &str) -> String {
    if cfg!(windows) {
        format!("'{}'", s.replace('\'', "''"))
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// The app's executable at a path that still works after this run ends, with
/// a warning when there is none.
fn stable_exe() -> Result<(PathBuf, Option<String>), String> {
    let exe = std::env::current_exe().map_err(es)?;
    // An AppImage runs from a temporary mount. $APPIMAGE is the file itself.
    if let Some(appimage) = std::env::var_os("APPIMAGE").map(PathBuf::from) {
        if appimage.is_file() {
            return Ok((appimage, None));
        }
    }
    let text = exe.to_string_lossy();
    if text.contains("/AppTranslocation/") || text.starts_with("/Volumes/") {
        let installed = Path::new("/Applications/Satchel.app/Contents/MacOS/satchel-app");
        if installed.is_file() {
            return Ok((installed.to_path_buf(), None));
        }
        return Ok((
            exe,
            Some(
                "Satchel is running from the disk image or a temporary location, so this command \
                 stops working when it quits. Move Satchel to Applications, open it from there, and \
                 copy the command again."
                    .into(),
            ),
        ));
    }
    Ok((exe, None))
}

#[tauri::command(async)]
pub fn agent_setup(state: State<AppState>) -> Result<AgentSetup, String> {
    let root = with_vault(&state, |v| Ok(v.root.clone()))?;
    let (exe, warning) = stable_exe()?;
    let (exe, root) = (exe.to_string_lossy(), root.to_string_lossy());
    // JSON string literals are valid TOML basic strings (escapes included).
    let toml = |s: &str| serde_json::to_string(s).unwrap_or_default();
    Ok(AgentSetup {
        claude_code: format!(
            "claude mcp add --transport stdio satchel -- {} mcp --vault {}",
            shell_quote(&exe),
            shell_quote(&root)
        ),
        codex: format!(
            "[mcp_servers.satchel]\ncommand = {}\nargs = [\"mcp\", \"--vault\", {}]\n",
            toml(&exe),
            toml(&root)
        ),
        warning,
    })
}

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
    // A cloud-only note is downloaded when read; do that without holding the
    // vault lock, then index it now that its content is local.
    let was_cloud_only = !abs.exists()
        || fs::metadata(&abs).map(|m| vault::is_dataless(&m)).unwrap_or(false);
    vault::ensure_local(&abs, Duration::from_secs(30)).map_err(es)?;
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

/// Re-index one note after an in-app change.
fn reindex_one(v: &mut Vault, rel_path: &str) -> Result<(), String> {
    let root = v.root.clone();
    index::index_single(&v.conn, &root, rel_path).map_err(es)?;
    Ok(())
}

/// Have the background indexer embed changed notes, if semantic search is in
/// use this session. A save never waits on the model this way.
fn refresh_embeddings<R: Runtime>(app: &AppHandle<R>, state: &State<AppState>) {
    if state.loaded_embedder().is_some() {
        spawn_semantic_indexer(app.clone());
    }
}

#[tauri::command(async)]
pub fn write_note<R: Runtime>(
    app: AppHandle<R>,
    state: State<AppState>,
    rel_path: String,
    content: String,
) -> Result<(), String> {
    with_vault(&state, |v| {
        let abs = vault::safe_join(&v.root, &rel_path).map_err(es)?;
        vault::write_note_atomic(&abs, &content).map_err(es)?;
        reindex_one(v, &rel_path)
    })?;
    refresh_embeddings(&app, &state);
    Ok(())
}

/// Split `rel_path` into its folder (if any), file stem and extension.
fn split_name(rel_path: &str) -> (Option<&str>, &str, &str) {
    let (dir, file) = match rel_path.rsplit_once('/') {
        Some((dir, file)) => (Some(dir), file),
        None => (None, rel_path),
    };
    match file.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (dir, stem, ext),
        _ => (dir, file, ""),
    }
}

/// Create a file at `rel_path`, or at "<stem> 1", "<stem> 2", … if that name
/// is taken, without ever replacing an existing file. Characters that are not
/// allowed in file names are replaced, and a leading dot (which would hide the
/// file) is dropped. `content` gets the final stem. Returns the path used.
fn create_unique(root: &Path, rel_path: &str, content: impl Fn(&str) -> String) -> Result<String, String> {
    let (dir, stem, ext) = split_name(rel_path);
    let stem = vault::note_file_name(stem);
    let ext = if ext.is_empty() { String::new() } else { format!(".{ext}") };
    for n in 0..10_000 {
        let name = if n == 0 { stem.clone() } else { format!("{stem} {n}") };
        let rel = match dir {
            Some(dir) => format!("{dir}/{name}{ext}"),
            None => format!("{name}{ext}"),
        };
        if vault::is_hidden_path(&rel) {
            return Err("notes cannot go in hidden folders".into());
        }
        let abs = vault::safe_join(root, &rel).map_err(es)?;
        match vault::create_file_exclusive(&abs, content(&name).as_bytes()) {
            Ok(()) => return Ok(rel),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(es(e)),
        }
    }
    Err(format!("too many notes named \"{stem}\""))
}

#[tauri::command(async)]
pub fn create_note<R: Runtime>(
    app: AppHandle<R>,
    state: State<AppState>,
    rel_path: String,
    content: Option<String>,
) -> Result<String, String> {
    // The heading keeps the name as typed (say "Plan: Q3"), even if the file
    // name had to change, so a [[Plan: Q3]] link still finds the note.
    let (_, typed, _) = split_name(&rel_path);
    let typed = typed.to_string();
    let rel = with_vault(&state, |v| {
        let root = v.root.clone();
        let rel = create_unique(&root, &rel_path, |stem| match &content {
            Some(body) => body.clone(),
            None if vault::note_file_name(&typed) == stem => format!("# {typed}\n\n"),
            None => format!("# {stem}\n\n"),
        })?;
        reindex_one(v, &rel)?;
        Ok(rel)
    })?;
    refresh_embeddings(&app, &state);
    Ok(rel)
}

/// The file to move for the note at `abs`: the note itself, or the macOS 13
/// iCloud placeholder standing in for it, with where that should go.
fn movable(abs: &Path, to: &Path) -> Option<(PathBuf, PathBuf)> {
    if abs.is_file() {
        return Some((abs.to_path_buf(), to.to_path_buf()));
    }
    let placeholder = vault::icloud_placeholder(abs)?;
    let name = to.file_name()?.to_string_lossy();
    Some((placeholder, to.with_file_name(format!(".{name}.icloud"))))
}

/// Rename or move a note. Its index entry moves with it, and `[[links]]` to
/// it in other notes are rewritten when they named the old file. Returns how
/// many notes had links rewritten.
#[tauri::command(async)]
pub fn rename_note<R: Runtime>(
    app: AppHandle<R>,
    state: State<AppState>,
    from: String,
    to: String,
) -> Result<usize, String> {
    require_subpath(&from)?;
    require_subpath(&to)?;
    if vault::is_hidden_path(&to) {
        return Err("notes cannot go in hidden folders".into());
    }
    let rewritten = with_vault(&state, |v| {
        let abs_from = vault::safe_join(&v.root, &from).map_err(es)?;
        let abs_to = vault::safe_join(&v.root, &to).map_err(es)?;
        let Some((src, dst)) = movable(&abs_from, &abs_to) else {
            return Err(format!("\"{from}\" is not a note"));
        };
        if target_taken(&abs_from, &abs_to) {
            return Err(format!("\"{to}\" already exists"));
        }
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(es)?;
        }
        let before = match index::note_id_by_path(&v.conn, &from).map_err(es)? {
            Some(id) => index::inbound_links(&v.conn, &[id]).map_err(es)?,
            None => Vec::new(),
        };
        fs::rename(&src, &dst).map_err(es)?;
        let root = v.root.clone();
        index::rename_note(&v.conn, &root, &from, &to).map_err(es)?;
        let rewritten = index::rewrite_moved_links(&v.conn, &root, &before).map_err(es)?;
        Ok(rewritten.len())
    })?;
    refresh_embeddings(&app, &state);
    Ok(rewritten)
}

/// Move a note to the system Trash (recoverable), then drop it from the index.
#[tauri::command(async)]
pub fn delete_note(state: State<AppState>, rel_path: String) -> Result<(), String> {
    require_subpath(&rel_path)?;
    let root = with_vault(&state, |v| Ok(v.root.clone()))?;
    let abs = vault::safe_join(&root, &rel_path).map_err(es)?;
    if abs.is_dir() {
        return Err("not a note".into());
    }
    // Trashing can be slow (a large note on a cloud drive), so it runs without
    // the vault lock.
    if abs.is_file() {
        move_to_trash(&abs)?;
    } else if let Some(placeholder) = vault::icloud_placeholder(&abs) {
        move_to_trash(&placeholder)?;
    }
    with_vault(&state, |v| {
        if v.root == root {
            reindex_one(v, &rel_path)?;
        }
        Ok(())
    })
}

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

/// Rename or move a folder. Its notes keep their index entries, and links
/// that named them by path are rewritten. Returns how many notes had links
/// rewritten.
#[tauri::command(async)]
pub fn rename_folder(state: State<AppState>, from: String, to: String) -> Result<usize, String> {
    require_subpath(&from)?;
    require_subpath(&to)?;
    if vault::is_hidden_path(&to) {
        return Err("folders cannot be hidden".into());
    }
    with_vault(&state, |v| {
        let abs_from = vault::safe_join(&v.root, &from).map_err(es)?;
        let abs_to = vault::safe_join(&v.root, &to).map_err(es)?;
        if !abs_from.is_dir() {
            return Err(format!("\"{from}\" is not a folder"));
        }
        if target_taken(&abs_from, &abs_to) {
            return Err(format!("\"{to}\" already exists"));
        }
        if abs_to.starts_with(&abs_from) && abs_to != abs_from {
            return Err("cannot move a folder into itself".into());
        }
        if let Some(parent) = abs_to.parent() {
            fs::create_dir_all(parent).map_err(es)?;
        }
        let ids = index::note_ids_in_folder(&v.conn, &from).map_err(es)?;
        let before = index::inbound_links(&v.conn, &ids).map_err(es)?;
        fs::rename(&abs_from, &abs_to).map_err(es)?;
        let root = v.root.clone();
        index::rename_folder(&v.conn, &from, &to).map_err(es)?;
        let rewritten = index::rewrite_moved_links(&v.conn, &root, &before).map_err(es)?;
        Ok(rewritten.len())
    })
}

/// Move a folder (and everything in it) to the system Trash.
#[tauri::command(async)]
pub fn delete_folder(state: State<AppState>, rel_path: String) -> Result<(), String> {
    require_subpath(&rel_path)?;
    let root = with_vault(&state, |v| Ok(v.root.clone()))?;
    let abs = vault::safe_join(&root, &rel_path).map_err(es)?;
    if !abs.is_dir() {
        return Err(format!("\"{rel_path}\" is not a folder"));
    }
    // A big folder can take a while to trash, so this runs without the lock.
    move_to_trash(&abs)?;
    with_vault(&state, |v| {
        if v.root == root {
            index::remove_folder(&v.conn, &rel_path).map_err(es)?;
        }
        Ok(())
    })
}

// ---- attachments -----------------------------------------------------------

/// Store a pasted or dropped file in the vault's `attachments/` folder,
/// named by its content hash so the same image is stored once. The bytes
/// arrive as the raw request body (not a JSON array of numbers) and the file
/// extension in the `x-satchel-ext` header. Returns its vault path.
#[tauri::command(async)]
pub fn save_attachment(request: Request<'_>, state: State<AppState>) -> Result<String, String> {
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err("expected the file's bytes".into());
    };
    let ext = request
        .headers()
        .get("x-satchel-ext")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let safe_ext: String = ext.chars().filter(|c| c.is_alphanumeric()).take(8).collect();
    let safe_ext = if safe_ext.is_empty() {
        "bin".to_string()
    } else {
        safe_ext.to_lowercase()
    };
    let root = with_vault(&state, |v| Ok(v.root.clone()))?;
    let rel = format!("attachments/{}.{safe_ext}", vault::content_hash(bytes));
    let abs = vault::safe_join(&root, &rel).map_err(es)?;
    if !abs.exists() {
        vault::write_bytes_atomic(&abs, bytes).map_err(es)?;
    }
    Ok(rel)
}

/// Read a file a note embeds, as raw bytes. `target` is resolved next to
/// `from_note`, then from the vault root, then (a bare file name) anywhere in
/// the vault. Runs without the vault lock, as a cloud-only file may first
/// have to download.
#[tauri::command(async)]
pub fn read_attachment(
    state: State<AppState>,
    target: String,
    from_note: Option<String>,
) -> Result<Response, String> {
    let root = with_vault(&state, |v| Ok(v.root.clone()))?;
    let abs = vault::resolve_attachment(&root, from_note.as_deref(), &target).map_err(es)?;
    Ok(Response::new(fs::read(&abs).map_err(es)?))
}

// ---- canvas (JSONCanvas) ---------------------------------------------------

#[tauri::command(async)]
pub fn list_canvases(state: State<AppState>) -> Result<Vec<String>, String> {
    with_vault(&state, |v| Ok(vault::list_by_ext(&v.root, "canvas")))
}

#[tauri::command(async)]
pub fn read_canvas(state: State<AppState>, rel_path: String) -> Result<Canvas, String> {
    let root = with_vault(&state, |v| Ok(v.root.clone()))?;
    let abs = vault::safe_join(&root, &rel_path).map_err(es)?;
    let json = if abs.exists() {
        vault::read_note(&abs).map_err(es)?
    } else {
        String::new()
    };
    canvas::parse_canvas(&json).map_err(es)
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
    let mut rel = rel_path;
    if !rel.to_lowercase().ends_with(".canvas") {
        rel.push_str(".canvas");
    }
    let json = canvas::serialize_canvas(&Canvas::default()).map_err(es)?;
    with_vault(&state, |v| create_unique(&v.root, &rel, |_| json.clone()))
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

/// Embed a search query. Runs outside the vault lock, so a search that waits
/// for the model (loading it, or the background indexer using it) does not
/// hold up saves and other commands.
fn embed_query(state: &State<AppState>, query: &str) -> Result<Vec<f32>, String> {
    let embedder = state.embedder()?;
    embedder.embed_one(query).map_err(es)
}

#[tauri::command(async)]
pub fn search_semantic(
    state: State<AppState>,
    query: String,
    limit: usize,
) -> Result<Vec<SearchHit>, String> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let qvec = embed_query(&state, &query)?;
    with_vault(&state, |v| {
        semantic::search_semantic_vec(&v.conn, &qvec, limit).map_err(es)
    })
}

#[tauri::command(async)]
pub fn search_hybrid(
    state: State<AppState>,
    query: String,
    limit: usize,
) -> Result<Vec<SearchHit>, String> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let qvec = embed_query(&state, &query)?;
    with_vault(&state, |v| {
        semantic::search_hybrid_vec(&v.conn, &query, &qvec, limit).map_err(es)
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

        // Unlinked mentions: notes that mention the title as a whole phrase
        // but do not link here.
        for (rel_path, title) in index::unlinked_mentions(&v.conn, note_id, 50).map_err(es)? {
            linked.push(Backlink { rel_path, title, unlinked: true });
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

// ---- quitting --------------------------------------------------------------

/// Event asking the window to save unsaved edits before the app quits.
pub const APP_EXIT_REQUESTED: &str = "app-exit-requested";

/// Quit once the window has saved its unsaved edits (it answers with
/// `confirm_exit`), or after a few seconds if it does not answer.
pub fn request_exit<R: Runtime>(app: &AppHandle<R>) {
    if app.state::<AppState>().exiting.swap(true, Ordering::SeqCst) {
        return;
    }
    if app.emit(APP_EXIT_REQUESTED, ()).is_err() {
        app.exit(0);
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(3));
        app.exit(0);
    });
}

/// The window has saved its edits, so quit now.
#[tauri::command(async)]
pub fn confirm_exit<R: Runtime>(app: AppHandle<R>) {
    app.exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quoting_keeps_paths_literal() {
        if cfg!(windows) {
            assert_eq!(shell_quote("C:\\it's here"), "'C:\\it''s here'");
        } else {
            assert_eq!(shell_quote("/Notes $work/`x`"), "'/Notes $work/`x`'");
            assert_eq!(shell_quote("it's"), "'it'\\''s'");
        }
    }

    #[test]
    fn new_notes_get_unique_safe_names() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let body = |stem: &str| format!("# {stem}\n");
        assert_eq!(create_unique(root, "v1.2/Draft.md", body).unwrap(), "v1.2/Draft.md");
        assert_eq!(create_unique(root, "v1.2/Draft.md", body).unwrap(), "v1.2/Draft 1.md");
        assert_eq!(fs::read_to_string(root.join("v1.2/Draft 1.md")).unwrap(), "# Draft 1\n");
        assert_eq!(create_unique(root, ".env.md", body).unwrap(), "env.md");
        assert_eq!(create_unique(root, "Plan: Q3.md", body).unwrap(), "Plan- Q3.md");
        assert!(create_unique(root, ".obsidian/x.md", body).is_err());
    }

    #[test]
    fn a_rename_never_replaces_a_different_file() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.md"), dir.path().join("b.md"));
        fs::write(&a, "a").unwrap();
        fs::write(&b, "b").unwrap();
        assert!(target_taken(&a, &b));
        assert!(!target_taken(&a, &a));
        assert!(!target_taken(&a, &dir.path().join("c.md")));
    }
}
