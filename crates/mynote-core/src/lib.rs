//! # mynote-core
//!
//! The shared core of MyNote. Files (Markdown + attachments in a user-chosen vault) are the
//! source of truth; a local SQLite database is a *derived, rebuildable index*. This crate holds
//! all logic — vault I/O, parsing, indexing, search (full-text + semantic), and the read-only
//! query service — and is consumed by three thin shells: the Tauri desktop app, the `mynote`
//! CLI, and the MCP server.
//!
//! Modules are added phase by phase (see `.forge/PLAN.md`).

/// Re-exported so shells (app/CLI/MCP) can name connection types without
/// declaring their own `rusqlite` dependency (and version skew).
pub use rusqlite;

pub mod agents;
pub mod canvas;
pub mod db;
pub mod embed;
pub mod index;
pub mod model;
pub mod parse;
pub mod query;
pub mod search;
pub mod vault;
pub mod watch;

/// The crate version (from Cargo).
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_nonempty() {
        assert!(!version().is_empty(), "version should be a non-empty string");
    }
}

/// End-to-end exercise of the exact data path the desktop app's commands drive:
/// open → reindex → list → edit/save → re-index → full-text + query. This is the
/// headless proof behind Phase D/E (which have no automated GUI check).
#[cfg(test)]
mod flow {
    use crate::{db, index, query, search, vault};
    use std::fs;

    #[test]
    fn open_edit_save_search_query() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("welcome.md"), "# Welcome\n\nhello there [[Ideas]]").unwrap();
        fs::write(root.join("Ideas.md"), "# Ideas\n\nbrainstorm").unwrap();

        let db_path = root.join(".mynote/index.db");
        let conn = db::open(&db_path).unwrap();

        // open_vault: reindex + list
        index::reindex_all(&conn, root).unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM notes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);

        // read_note (from disk, source of truth)
        let original = vault::read_note(&root.join("welcome.md")).unwrap();
        assert!(original.contains("hello there"));

        // write_note: atomic write + single-note reindex (what the editor does)
        vault::write_note_atomic(
            &root.join("welcome.md"),
            "# Welcome\n\nupdated with the word zebra",
        )
        .unwrap();
        index::index_single(&conn, root, "welcome.md").unwrap();

        // full-text search reflects the edit
        let hits = search::fts::search_fulltext(&conn, "zebra", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].rel_path, "welcome.md");

        // sandboxed query over the index
        let qconn = query::open_query_connection(&db_path).unwrap();
        let result = query::run_query(&qconn, "SELECT count(*) FROM notes", 10).unwrap();
        assert_eq!(result.rows[0][0], serde_json::json!(2));
    }
}
