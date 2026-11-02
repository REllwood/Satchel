//! SQLite index: connection setup, schema, and migrations.
//!
//! The database (kept per machine, see [`crate::paths::index_db_path`]) is a
//! *derived cache* — it can be rebuilt from the vault at any time. It runs in
//! WAL mode so the app, the CLI and the MCP server can share it, and every
//! multi-step write goes through [`atomically`].

use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use rusqlite::Connection;

pub mod vec;

/// Bump when the schema changes; `migrate` applies steps up to this.
pub const SCHEMA_VERSION: i64 = 1;

const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
  key   TEXT PRIMARY KEY,
  value TEXT
);

CREATE TABLE IF NOT EXISTS notes (
  id           INTEGER PRIMARY KEY,
  rel_path     TEXT NOT NULL UNIQUE,
  title        TEXT NOT NULL DEFAULT '',
  frontmatter  TEXT NOT NULL DEFAULT 'null',
  body         TEXT NOT NULL DEFAULT '',
  plaintext    TEXT NOT NULL DEFAULT '',
  mtime        INTEGER NOT NULL DEFAULT 0,
  size         INTEGER NOT NULL DEFAULT 0,
  hash         TEXT NOT NULL DEFAULT '',
  materialized INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS idx_notes_hash ON notes(hash);

CREATE TABLE IF NOT EXISTS links (
  id             INTEGER PRIMARY KEY,
  src_note_id    INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
  target         TEXT NOT NULL,
  heading        TEXT,
  alias          TEXT,
  kind           TEXT NOT NULL DEFAULT 'wikilink',
  target_note_id INTEGER REFERENCES notes(id) ON DELETE SET NULL,
  resolved       INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_links_src ON links(src_note_id);
CREATE INDEX IF NOT EXISTS idx_links_target ON links(target);
CREATE INDEX IF NOT EXISTS idx_links_target_note ON links(target_note_id);

CREATE TABLE IF NOT EXISTS tags (
  note_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
  tag     TEXT NOT NULL,
  PRIMARY KEY (note_id, tag)
);
CREATE INDEX IF NOT EXISTS idx_tags_tag ON tags(tag);

CREATE TABLE IF NOT EXISTS attachments (
  id       INTEGER PRIMARY KEY,
  note_id  INTEGER REFERENCES notes(id) ON DELETE CASCADE,
  rel_path TEXT NOT NULL,
  mime     TEXT
);
CREATE INDEX IF NOT EXISTS idx_attachments_note ON attachments(note_id);

CREATE TABLE IF NOT EXISTS chunks (
  id      INTEGER PRIMARY KEY,
  note_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
  ord     INTEGER NOT NULL DEFAULT 0,
  start   INTEGER NOT NULL DEFAULT 0,
  "end"   INTEGER NOT NULL DEFAULT 0,
  text    TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_chunks_note ON chunks(note_id);

-- External-content FTS5: bodies live in `notes`, not duplicated here.
CREATE VIRTUAL TABLE IF NOT EXISTS note_fts USING fts5(
  title, plaintext,
  content='notes', content_rowid='id',
  tokenize='porter unicode61', prefix='2 3'
);

CREATE TRIGGER IF NOT EXISTS notes_ai AFTER INSERT ON notes BEGIN
  INSERT INTO note_fts(rowid, title, plaintext) VALUES (new.id, new.title, new.plaintext);
END;
CREATE TRIGGER IF NOT EXISTS notes_ad AFTER DELETE ON notes BEGIN
  INSERT INTO note_fts(note_fts, rowid, title, plaintext) VALUES ('delete', old.id, old.title, old.plaintext);
END;
CREATE TRIGGER IF NOT EXISTS notes_au AFTER UPDATE ON notes BEGIN
  INSERT INTO note_fts(note_fts, rowid, title, plaintext) VALUES ('delete', old.id, old.title, old.plaintext);
  INSERT INTO note_fts(rowid, title, plaintext) VALUES (new.id, new.title, new.plaintext);
END;
"#;

/// Open (creating if needed) the index DB at `path`, configure pragmas, register
/// sqlite-vec, and run migrations.
pub fn open(path: &Path) -> Result<Connection> {
    vec::ensure_registered();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    configure(&conn)?;
    migrate(&conn)?;
    Ok(conn)
}

/// In-memory database (tests, ephemeral use).
pub fn open_in_memory() -> Result<Connection> {
    vec::ensure_registered();
    let conn = Connection::open_in_memory()?;
    configure(&conn)?;
    migrate(&conn)?;
    Ok(conn)
}

fn configure(conn: &Connection) -> Result<()> {
    conn.busy_timeout(Duration::from_secs(5))?;
    // journal_mode returns the resulting mode as a row, so query rather than update.
    let _mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    Ok(())
}

fn migrate(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < 1 {
        conn.execute_batch(SCHEMA_V1)?;
    }
    // Future migrations: `if version < 2 { ... }`
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

/// Run `f` so that its writes land together or not at all. The outermost call
/// opens a transaction that takes the write lock at the start, so it cannot
/// fail half-way for want of the lock. A call made inside it uses a savepoint,
/// so an error undoes only that call's writes and the outer work goes on.
pub fn atomically<T>(conn: &Connection, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let outer = conn.is_autocommit();
    conn.execute_batch(if outer { "BEGIN IMMEDIATE" } else { "SAVEPOINT atomically" })?;
    let mut undo = Undo { conn, outer, armed: true };
    let value = f()?;
    conn.execute_batch(if outer { "COMMIT" } else { "RELEASE atomically" })?;
    undo.armed = false;
    Ok(value)
}

/// Rolls back an [`atomically`] block that returned an error or panicked.
struct Undo<'a> {
    conn: &'a Connection,
    outer: bool,
    armed: bool,
}

impl Drop for Undo<'_> {
    fn drop(&mut self) {
        if self.armed {
            let sql = if self.outer {
                "ROLLBACK"
            } else {
                "ROLLBACK TO atomically; RELEASE atomically"
            };
            let _ = self.conn.execute_batch(sql);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_exists(conn: &Connection, name: &str) -> bool {
        conn.query_row(
            "SELECT 1 FROM sqlite_master WHERE name = ?1",
            [name],
            |_| Ok(()),
        )
        .is_ok()
    }

    #[test]
    fn migrate_creates_all_tables_and_sets_version() {
        let conn = open_in_memory().unwrap();
        for t in [
            "meta", "notes", "links", "tags", "attachments", "chunks", "note_fts",
        ] {
            assert!(table_exists(&conn, t), "missing table {t}");
        }
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
    }

    #[test]
    fn file_db_uses_wal_and_reopen_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".satchel/index.db");
        {
            let conn = open(&path).unwrap();
            let mode: String = conn
                .query_row("PRAGMA journal_mode", [], |r| r.get(0))
                .unwrap();
            assert_eq!(mode.to_lowercase(), "wal");
        }
        // Reopen: migration is a no-op, tables persist.
        let conn = open(&path).unwrap();
        assert!(table_exists(&conn, "notes"));
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
    }

    #[test]
    fn fts_external_content_round_trips_via_triggers() {
        let conn = open_in_memory().unwrap();
        conn.execute(
            "INSERT INTO notes (rel_path, title, plaintext) VALUES ('a.md', 'Foxes', 'the quick brown fox jumps')",
            [],
        )
        .unwrap();
        let hits: i64 = conn
            .query_row(
                "SELECT count(*) FROM note_fts WHERE note_fts MATCH 'quick'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 1, "FTS trigger should index the inserted note");
    }

    fn paths(conn: &Connection) -> Vec<String> {
        let mut stmt = conn.prepare("SELECT rel_path FROM notes ORDER BY rel_path").unwrap();
        stmt.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect()
    }

    fn insert(conn: &Connection, rel: &str) -> Result<()> {
        conn.execute("INSERT INTO notes (rel_path) VALUES (?1)", [rel])?;
        Ok(())
    }

    #[test]
    fn atomically_commits_or_undoes_all_of_its_writes() {
        let conn = open_in_memory().unwrap();
        atomically(&conn, || insert(&conn, "a.md")).unwrap();
        let failed = atomically(&conn, || -> Result<()> {
            insert(&conn, "b.md")?;
            anyhow::bail!("stopped half-way")
        });
        assert!(failed.is_err());
        assert_eq!(paths(&conn), ["a.md"]);
        assert!(conn.is_autocommit(), "no transaction is left open");
    }

    #[test]
    fn a_failed_inner_block_undoes_only_its_own_writes() {
        let conn = open_in_memory().unwrap();
        atomically(&conn, || {
            insert(&conn, "a.md")?;
            let inner = atomically(&conn, || {
                insert(&conn, "b.md")?;
                insert(&conn, "a.md") // duplicate path: fails
            });
            assert!(inner.is_err());
            insert(&conn, "c.md")
        })
        .unwrap();
        assert_eq!(paths(&conn), ["a.md", "c.md"]);
        assert!(conn.is_autocommit());
    }

    #[test]
    fn a_panic_rolls_back() {
        let conn = open_in_memory().unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            atomically(&conn, || -> Result<()> {
                insert(&conn, "a.md")?;
                panic!("bug")
            })
        }));
        assert!(result.is_err());
        assert!(paths(&conn).is_empty());
        assert!(conn.is_autocommit());
    }

    #[test]
    fn a_writer_waits_for_another_connections_transaction() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let first = open(&path).unwrap();
        let second = open(&path).unwrap();
        second.busy_timeout(Duration::from_millis(50)).unwrap();
        atomically(&first, || {
            insert(&first, "a.md")?;
            // The second writer cannot start while the first holds the lock.
            assert!(atomically(&second, || insert(&second, "b.md")).is_err());
            Ok(())
        })
        .unwrap();
        atomically(&second, || insert(&second, "b.md")).unwrap();
        assert_eq!(paths(&first), ["a.md", "b.md"]);
    }
}
