//! sqlite-vec integration: register the extension once per process and manage
//! the `vec_chunks` virtual table used for on-device semantic search.
//!
//! Registering via `sqlite3_auto_extension` means the extension is statically
//! linked into the binary — no per-platform `.dylib` to ship.

use std::sync::Once;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};

static REGISTER: Once = Once::new();

/// Register sqlite-vec for all subsequent connections in this process. Safe to
/// call repeatedly; only the first call has effect.
pub fn ensure_registered() {
    REGISTER.call_once(|| unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    });
}

/// Pack `f32`s into the little-endian byte layout sqlite-vec expects for a
/// `FLOAT[N]` column.
pub fn f32_blob(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

/// Create the `vec_chunks` table sized to `dim` (idempotent).
pub fn create_vec_table(conn: &Connection, dim: u32) -> Result<()> {
    conn.execute_batch(&format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS vec_chunks USING vec0(\
            chunk_id INTEGER PRIMARY KEY, embedding FLOAT[{dim}]\
        );"
    ))?;
    Ok(())
}

/// sqlite-vec version string (e.g. "v0.1.9").
pub fn version(conn: &Connection) -> Result<String> {
    Ok(conn.query_row("SELECT vec_version()", [], |r| r.get(0))?)
}

/// Delete all vectors belonging to a note's chunks. Tolerant of the
/// `vec_chunks` table not existing yet (embeddings may never have run).
pub fn delete_note_vectors(conn: &Connection, note_id: i64) -> Result<()> {
    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE name = 'vec_chunks'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if exists {
        conn.execute(
            "DELETE FROM vec_chunks WHERE chunk_id IN (SELECT id FROM chunks WHERE note_id = ?1)",
            [note_id],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[test]
    fn vec_extension_loads_and_reports_version() {
        let conn = db::open_in_memory().unwrap();
        let v = version(&conn).unwrap();
        assert!(v.starts_with('v'), "unexpected vec_version: {v}");
    }

    #[test]
    fn knn_returns_nearest_vector() {
        let conn = db::open_in_memory().unwrap();
        create_vec_table(&conn, 4).unwrap();
        conn.execute(
            "INSERT INTO vec_chunks(chunk_id, embedding) VALUES (1, ?1)",
            rusqlite::params![f32_blob(&[1.0, 0.0, 0.0, 0.0])],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO vec_chunks(chunk_id, embedding) VALUES (2, ?1)",
            rusqlite::params![f32_blob(&[0.0, 1.0, 0.0, 0.0])],
        )
        .unwrap();

        let query = f32_blob(&[0.9, 0.1, 0.0, 0.0]);
        let nearest: i64 = conn
            .query_row(
                "SELECT chunk_id FROM vec_chunks WHERE embedding MATCH ?1 AND k = 1",
                rusqlite::params![query],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(nearest, 1, "vector 1 is closest to the query");
    }
}
