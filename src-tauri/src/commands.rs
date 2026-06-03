//! Tauri command surface. Thin wrappers over `mynote-core`; all errors are
//! mapped to strings at this boundary. The frontend only ever talks to these.

use std::fs;
use std::path::PathBuf;

use mynote_core::canvas::{self, Canvas};
use mynote_core::embed::{default_model_dir, Embedder};
use mynote_core::index::{self, IndexStats};
use mynote_core::query::{self, QueryResult};
use mynote_core::search::{self, semantic, SearchHit};
use mynote_core::{db, rusqlite, vault, watch};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::state::{AppState, Vault};

fn es<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[derive(Serialize)]
pub struct VaultInfo {
    pub root: String,
    pub note_count: i64,
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

// ---- helpers ---------------------------------------------------------------

fn with_vault<T>(
    state: &State<AppState>,
    f: impl FnOnce(&mut Vault) -> Result<T, String>,
) -> Result<T, String> {
    let mut guard = state.vault.lock().map_err(|_| "state poisoned")?;
    let vault = guard.as_mut().ok_or("no vault is open")?;
    f(vault)
}

/// Ensure the embedder is loaded (model load is lazy). After this returns Ok,
/// `vault.embedder` is `Some`. Returns `()` so the caller can take disjoint
/// immutable borrows of `vault.conn` and `vault.embedder`.
fn ensure_embedder(vault: &mut Vault) -> Result<(), String> {
    if vault.embedder.is_none() {
        vault.embedder = Some(Embedder::from_dir(&default_model_dir()).map_err(es)?);
    }
    Ok(())
}

fn note_count(conn: &rusqlite::Connection) -> Result<i64, String> {
    conn.query_row("SELECT count(*) FROM notes", [], |r| r.get(0))
        .map_err(es)
}

fn last_vault_file(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|d| d.join("last_vault.txt"))
}

// ---- vault lifecycle -------------------------------------------------------

#[tauri::command]
pub fn open_vault(
    app: AppHandle,
    state: State<AppState>,
    path: String,
) -> Result<VaultInfo, String> {
    let root = PathBuf::from(&path);
    if !root.is_dir() {
        return Err(format!("not a directory: {path}"));
    }
    let db_path = root.join(".mynote/index.db");
    let conn = db::open(&db_path).map_err(es)?;
    index::reindex_all(&conn, &root).map_err(es)?;
    // Drop agent-convention docs (AGENTS.md/CLAUDE.md) if absent; best-effort.
    let _ = mynote_core::agents::ensure_agent_docs(&root);
    let query_conn = query::open_query_connection(&db_path).map_err(es)?;
    let count = note_count(&conn)?;

    // Watch for external edits (AI tools, other editors, cloud sync): reconcile
    // the index and notify the frontend. Own-writes are idempotent re-indexes.
    let watcher = {
        let app = app.clone();
        watch::start(&root, move |events| {
            if let Some(state) = app.try_state::<AppState>() {
                if let Ok(mut guard) = state.vault.lock() {
                    if let Some(v) = guard.as_mut() {
                        for ev in &events {
                            let _ = index::index_single(&v.conn, &v.root, &ev.rel_path);
                            if !ev.removed && v.embedder.is_some() {
                                if let Ok(Some(id)) =
                                    index::note_id_by_path(&v.conn, &ev.rel_path)
                                {
                                    let emb = v.embedder.as_ref().unwrap();
                                    let _ =
                                        semantic::index_note_embeddings(&v.conn, emb, id);
                                }
                            }
                        }
                    }
                }
            }
            let _ = app.emit("vault-changed", events);
        })
        .ok()
    };

    *state.vault.lock().map_err(|_| "state poisoned")? = Some(Vault {
        root: root.clone(),
        conn,
        query_conn,
        embedder: None,
        watch: watcher,
    });

    if let Some(file) = last_vault_file(&app) {
        if let Some(parent) = file.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(file, &path);
    }

    Ok(VaultInfo {
        root: path,
        note_count: count,
    })
}

#[tauri::command]
pub fn current_vault(state: State<AppState>) -> Result<Option<VaultInfo>, String> {
    let guard = state.vault.lock().map_err(|_| "state poisoned")?;
    match guard.as_ref() {
        Some(v) => Ok(Some(VaultInfo {
            root: v.root.to_string_lossy().into_owned(),
            note_count: note_count(&v.conn)?,
        })),
        None => Ok(None),
    }
}

#[tauri::command]
pub fn close_vault(state: State<AppState>) -> Result<(), String> {
    *state.vault.lock().map_err(|_| "state poisoned")? = None;
    Ok(())
}

#[tauri::command]
pub fn get_last_vault(app: AppHandle) -> Option<String> {
    last_vault_file(&app)
        .and_then(|f| fs::read_to_string(f).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[tauri::command]
pub fn reindex(state: State<AppState>) -> Result<IndexStats, String> {
    with_vault(&state, |v| {
        let root = v.root.clone();
        index::reindex_all(&v.conn, &root).map_err(es)
    })
}

#[tauri::command]
pub fn embed_vault(state: State<AppState>) -> Result<usize, String> {
    with_vault(&state, |v| {
        ensure_embedder(v)?;
        let embedder = v.embedder.as_ref().unwrap();
        semantic::embed_all(&v.conn, embedder).map_err(es)
    })
}

// ---- notes -----------------------------------------------------------------

#[tauri::command]
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

#[tauri::command]
pub fn read_note(state: State<AppState>, rel_path: String) -> Result<NoteContent, String> {
    with_vault(&state, |v| {
        let abs = vault::safe_join(&v.root, &rel_path).map_err(es)?;
        let content = vault::read_note(&abs).map_err(es)?;
        Ok(NoteContent { rel_path, content })
    })
}

fn reindex_one(v: &mut Vault, rel_path: &str) -> Result<(), String> {
    let root = v.root.clone();
    let id = index::index_single(&v.conn, &root, rel_path).map_err(es)?;
    // Keep embeddings current if they've been built this session.
    if v.embedder.is_some() {
        if let Some(id) = id {
            let embedder = v.embedder.as_ref().unwrap();
            semantic::index_note_embeddings(&v.conn, embedder, id).map_err(es)?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn write_note(state: State<AppState>, rel_path: String, content: String) -> Result<(), String> {
    with_vault(&state, |v| {
        let abs = vault::safe_join(&v.root, &rel_path).map_err(es)?;
        vault::write_note_atomic(&abs, &content).map_err(es)?;
        reindex_one(v, &rel_path)
    })
}

#[tauri::command]
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

#[tauri::command]
pub fn rename_note(state: State<AppState>, from: String, to: String) -> Result<(), String> {
    with_vault(&state, |v| {
        let abs_from = vault::safe_join(&v.root, &from).map_err(es)?;
        let abs_to = vault::safe_join(&v.root, &to).map_err(es)?;
        if abs_to.exists() {
            return Err(format!("target already exists: {to}"));
        }
        if let Some(parent) = abs_to.parent() {
            fs::create_dir_all(parent).map_err(es)?;
        }
        fs::rename(&abs_from, &abs_to).map_err(es)?;
        index::remove_note(&v.conn, &from).map_err(es)?;
        reindex_one(v, &to)
    })
}

#[tauri::command]
pub fn delete_note(state: State<AppState>, rel_path: String) -> Result<(), String> {
    with_vault(&state, |v| {
        let abs = vault::safe_join(&v.root, &rel_path).map_err(es)?;
        if abs.exists() {
            fs::remove_file(&abs).map_err(es)?;
        }
        reindex_one(v, &rel_path)
    })
}

// ---- attachments -----------------------------------------------------------

#[tauri::command]
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

#[tauri::command]
pub fn read_attachment(state: State<AppState>, rel_path: String) -> Result<Vec<u8>, String> {
    with_vault(&state, |v| {
        let abs = vault::safe_join(&v.root, &rel_path).map_err(es)?;
        fs::read(&abs).map_err(es)
    })
}

// ---- canvas (JSONCanvas) ---------------------------------------------------

#[tauri::command]
pub fn list_canvases(state: State<AppState>) -> Result<Vec<String>, String> {
    with_vault(&state, |v| Ok(vault::list_by_ext(&v.root, "canvas")))
}

#[tauri::command]
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

#[tauri::command]
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

#[tauri::command]
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

#[tauri::command]
pub fn search_fulltext(
    state: State<AppState>,
    query: String,
    limit: usize,
) -> Result<Vec<SearchHit>, String> {
    with_vault(&state, |v| {
        search::fts::search_fulltext(&v.conn, &query, limit).map_err(es)
    })
}

#[tauri::command]
pub fn search_semantic(
    state: State<AppState>,
    query: String,
    limit: usize,
) -> Result<Vec<SearchHit>, String> {
    with_vault(&state, |v| {
        ensure_embedder(v)?;
        let embedder = v.embedder.as_ref().unwrap();
        semantic::search_semantic(&v.conn, embedder, &query, limit).map_err(es)
    })
}

#[tauri::command]
pub fn search_hybrid(
    state: State<AppState>,
    query: String,
    limit: usize,
) -> Result<Vec<SearchHit>, String> {
    with_vault(&state, |v| {
        ensure_embedder(v)?;
        let embedder = v.embedder.as_ref().unwrap();
        semantic::search_hybrid(&v.conn, embedder, &query, limit).map_err(es)
    })
}

#[tauri::command]
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

#[tauri::command]
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

#[tauri::command]
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

#[tauri::command]
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

#[tauri::command]
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
