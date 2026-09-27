//! Full-text search via SQLite FTS5.

use anyhow::Result;
use rusqlite::Connection;

use super::{HitSource, SearchHit};

/// Turn arbitrary user input into a safe FTS5 MATCH expression: keep
/// alphanumerics (Unicode) and spaces, treat the rest as separators, and make
/// the final term a prefix match for as-you-type behaviour.
pub fn sanitize_query(input: &str) -> String {
    let cleaned: String = input
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let terms: Vec<&str> = cleaned.split_whitespace().collect();
    if terms.is_empty() {
        return String::new();
    }
    // Quote each term so FTS5 keywords (AND/OR/NOT/NEAR) are treated literally;
    // make the final term a prefix match. Non-alphanumerics were already
    // stripped above, so there are no embedded quotes to escape.
    let last = terms.len() - 1;
    terms
        .iter()
        .enumerate()
        .map(|(i, t)| if i == last { format!("\"{t}\"*") } else { format!("\"{t}\"") })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Full-text search returning ranked hits with snippets.
pub fn search_fulltext(conn: &Connection, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
    let match_expr = sanitize_query(query);
    if match_expr.is_empty() {
        return Ok(Vec::new());
    }

    let mut stmt = conn.prepare(
        "SELECT n.id, n.rel_path, n.title,
                snippet(note_fts, 1, char(2), char(3), '…', 12) AS snip,
                bm25(note_fts) AS rank
         FROM note_fts
         JOIN notes n ON n.id = note_fts.rowid
         WHERE note_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2",
    )?;

    let hits = stmt
        .query_map(rusqlite::params![match_expr, limit as i64], |r| {
            let bm25: f64 = r.get(4)?;
            Ok(SearchHit {
                note_id: r.get(0)?,
                rel_path: r.get(1)?,
                title: r.get(2)?,
                snippet: r.get(3)?,
                // bm25 is lower-is-better; negate so higher = more relevant.
                score: -bm25,
                source: HitSource::FullText,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::index;
    use std::fs;

    fn indexed_vault() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("fox.md"),
            "# Fox\n\nThe quick brown fox jumps over the lazy dog.",
        )
        .unwrap();
        fs::write(
            dir.path().join("rust.md"),
            "# Rust\n\nOwnership and borrowing make memory safety tractable.",
        )
        .unwrap();
        let conn = db::open_in_memory().unwrap();
        index::reindex_all(&conn, dir.path()).unwrap();
        (dir, conn)
    }

    #[test]
    fn finds_literal_term_ranked_first() {
        let (_d, conn) = indexed_vault();
        let hits = search_fulltext(&conn, "quick", 10).unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].rel_path, "fox.md");
        assert!(hits[0].snippet.contains(super::super::MARK_START), "snippet has match markers");
        assert!(hits[0].clone().with_bracket_marks().snippet.contains("[quick]"));
    }

    #[test]
    fn prefix_matches_as_you_type() {
        let (_d, conn) = indexed_vault();
        let hits = search_fulltext(&conn, "own", 10).unwrap(); // -> own* matches "ownership"
        assert!(hits.iter().any(|h| h.rel_path == "rust.md"));
    }

    #[test]
    fn malicious_query_does_not_error() {
        let (_d, conn) = indexed_vault();
        // FTS operator soup must not produce a syntax error.
        let hits = search_fulltext(&conn, "\"a: b* (NOT) OR ^x\"", 10).unwrap();
        let _ = hits; // may be empty; must not panic/err
    }

    #[test]
    fn empty_query_returns_nothing() {
        let (_d, conn) = indexed_vault();
        assert!(search_fulltext(&conn, "   ", 10).unwrap().is_empty());
    }
}
