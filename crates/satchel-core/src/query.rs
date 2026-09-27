//! Read-only, sandboxed SQL query service for the in-app "query view".
//!
//! Three layers of defense so a user (or an AI agent via MCP) can run analytical
//! SELECTs over the index without any ability to mutate or exfiltrate:
//!   1. a `query_only` connection (SQLite rejects writes),
//!   2. an authorizer that allows only SELECT/READ/FUNCTION (denies
//!      INSERT/UPDATE/DELETE/DDL/PRAGMA/ATTACH/…),
//!   3. a post-prepare `Statement::readonly()` assertion.
//!
//! Results are capped at `max_rows`.

use std::path::Path;

use anyhow::{bail, Result};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::types::ValueRef;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

/// Tabular result of a user query.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
    /// True if more rows existed than `max_rows`.
    pub truncated: bool,
}

/// Allow only read actions; deny everything else (writes, DDL, PRAGMA, ATTACH…).
fn read_only_authorizer(ctx: AuthContext<'_>) -> Authorization {
    match ctx.action {
        AuthAction::Select | AuthAction::Recursive => Authorization::Allow,
        AuthAction::Read { .. } => Authorization::Allow,
        AuthAction::Function { .. } => Authorization::Allow,
        _ => Authorization::Deny,
    }
}

/// Open a dedicated, sandboxed connection for running user queries against the
/// index DB. Uses `query_only` + an authorizer (defense in depth) rather than a
/// read-only file handle, which avoids WAL read-only-open issues.
pub fn open_query_connection(path: &Path) -> Result<Connection> {
    crate::db::vec::ensure_registered();
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.pragma_update(None, "query_only", true)?;
    conn.authorizer(Some(read_only_authorizer))?;
    Ok(conn)
}

fn cell_to_json(value: ValueRef<'_>) -> serde_json::Value {
    use serde_json::Value;
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::from(i),
        ValueRef::Real(f) => Value::from(f),
        ValueRef::Text(t) => Value::from(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Value::from(format!("<blob {} bytes>", b.len())),
    }
}

/// Run a read-only SQL query, returning up to `max_rows` rows. A wall-clock
/// budget bounds runaway queries (recursive CTEs, cartesian products) so a
/// caller — including an AI agent over MCP — can't wedge CPU.
pub fn run_query(conn: &Connection, sql: &str, max_rows: usize) -> Result<QueryResult> {
    let start = std::time::Instant::now();
    // Best-effort: if the handler can't be installed the query still runs.
    let _ = conn.progress_handler(
        10_000,
        Some(move || start.elapsed() > std::time::Duration::from_secs(3)),
    );
    let result = run_query_inner(conn, sql, max_rows);
    let _ = conn.progress_handler::<fn() -> bool>(0, None);
    result
}

fn run_query_inner(conn: &Connection, sql: &str, max_rows: usize) -> Result<QueryResult> {
    let mut stmt = conn.prepare(sql)?;
    if !stmt.readonly() {
        bail!("only read-only SELECT queries are allowed");
    }
    let columns: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let col_count = columns.len();

    let mut rows_out: Vec<Vec<serde_json::Value>> = Vec::new();
    let mut truncated = false;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        if rows_out.len() >= max_rows {
            truncated = true;
            break;
        }
        let mut record = Vec::with_capacity(col_count);
        for i in 0..col_count {
            record.push(cell_to_json(row.get_ref(i)?));
        }
        rows_out.push(record);
    }

    Ok(QueryResult {
        columns,
        rows: rows_out,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db, index};
    use std::fs;

    fn populated_db() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            fs::write(
                dir.path().join(format!("n{i}.md")),
                format!("# Note {i}\n\nbody {i} #tag{i}"),
            )
            .unwrap();
        }
        let db_path = dir.path().join(".satchel/index.db");
        {
            let conn = db::open(&db_path).unwrap();
            index::reindex_all(&conn, dir.path()).unwrap();
        }
        (dir, db_path)
    }

    #[test]
    fn select_returns_rows() {
        let (_d, path) = populated_db();
        let conn = open_query_connection(&path).unwrap();
        let result = run_query(&conn, "SELECT count(*) AS n FROM notes", 100).unwrap();
        assert_eq!(result.columns, vec!["n".to_string()]);
        assert_eq!(result.rows[0][0], serde_json::json!(5));
    }

    #[test]
    fn writes_and_ddl_are_rejected() {
        let (_d, path) = populated_db();
        let conn = open_query_connection(&path).unwrap();
        for sql in [
            "INSERT INTO notes (rel_path) VALUES ('x.md')",
            "UPDATE notes SET title = 'hacked'",
            "DELETE FROM notes",
            "DROP TABLE notes",
            "CREATE TABLE evil (x)",
            "ATTACH DATABASE 'other.db' AS other",
            "PRAGMA query_only = OFF",
        ] {
            assert!(
                run_query(&conn, sql, 100).is_err(),
                "should reject: {sql}"
            );
        }
        // The data is intact after all rejected attempts.
        let result = run_query(&conn, "SELECT count(*) FROM notes", 100).unwrap();
        assert_eq!(result.rows[0][0], serde_json::json!(5));
    }

    #[test]
    fn results_are_capped_at_max_rows() {
        let (_d, path) = populated_db();
        let conn = open_query_connection(&path).unwrap();
        let result = run_query(&conn, "SELECT rel_path FROM notes ORDER BY rel_path", 2).unwrap();
        assert_eq!(result.rows.len(), 2);
        assert!(result.truncated);
    }

    #[test]
    fn can_read_schema_and_join() {
        let (_d, path) = populated_db();
        let conn = open_query_connection(&path).unwrap();
        // A more involved analytical query (tags join) still works read-only.
        let result = run_query(
            &conn,
            "SELECT n.title, t.tag FROM notes n JOIN tags t ON t.note_id = n.id ORDER BY n.title LIMIT 3",
            100,
        )
        .unwrap();
        assert_eq!(result.columns, vec!["title".to_string(), "tag".to_string()]);
        assert!(!result.rows.is_empty());
    }
}
