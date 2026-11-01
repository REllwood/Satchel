//! Full-text search via SQLite FTS5.

use anyhow::Result;
use rusqlite::Connection;

use super::{HitSource, SearchHit};

/// Turn arbitrary user input into a safe FTS5 MATCH expression: keep
/// alphanumerics (Unicode) and spaces, treat the rest as separators, and make
/// the final term a prefix match for as-you-type behaviour. Every term must
/// match.
pub fn sanitize_query(input: &str) -> String {
    match_expr(&terms(input), " ")
}

fn terms(input: &str) -> Vec<String> {
    input
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// Quote each term so FTS5 keywords (AND/OR/NOT/NEAR) are treated literally,
/// and make the final term a prefix match. Non-alphanumerics were already
/// stripped, so there are no embedded quotes to escape.
fn match_expr(terms: &[String], joiner: &str) -> String {
    let last = terms.len().saturating_sub(1);
    terms
        .iter()
        .enumerate()
        .map(|(i, t)| if i == last { format!("\"{t}\"*") } else { format!("\"{t}\"") })
        .collect::<Vec<_>>()
        .join(joiner)
}

/// True if `text` contains Chinese, Japanese or Korean characters. The FTS
/// tokenizer treats a run of them as one word, so words inside a sentence
/// cannot be found through the index.
pub(crate) fn has_cjk(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c as u32,
            0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF
            | 0xF900..=0xFAFF | 0x20000..=0x2FFFF)
    })
}

/// Full-text search returning ranked hits with snippets. Notes containing
/// every word come first. If those are fewer than `limit`, notes containing
/// some of the words follow, so a question such as "when does the backup
/// run" still finds the note about backups.
pub fn search_fulltext(conn: &Connection, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
    let limit = limit.clamp(1, 1000);
    if has_cjk(query) {
        return search_substring(conn, query, limit);
    }
    let terms = terms(query);
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let mut hits = run_match(conn, &match_expr(&terms, " "), limit)?;
    if terms.len() > 1 && hits.len() < limit {
        let seen: std::collections::HashSet<i64> = hits.iter().map(|h| h.note_id).collect();
        let more = run_match(conn, &match_expr(&terms, " OR "), limit)?;
        hits.extend(more.into_iter().filter(|h| !seen.contains(&h.note_id)));
        hits.truncate(limit);
    }
    Ok(hits)
}

/// Notes containing `phrase` as an exact phrase (no prefix matching), best
/// first. Used to find unlinked mentions of a note's title.
pub fn search_phrase(conn: &Connection, phrase: &str, limit: usize) -> Result<Vec<SearchHit>> {
    let terms = terms(phrase);
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let expr = format!("\"{}\"", terms.join(" "));
    run_match(conn, &expr, limit.clamp(1, 1000))
}

fn run_match(conn: &Connection, match_expr: &str, limit: usize) -> Result<Vec<SearchHit>> {
    let mut stmt = conn.prepare_cached(
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

/// Search by plain substring, for queries in scripts the tokenizer cannot
/// split into words. Every space-separated part of the query must appear.
fn search_substring(conn: &Connection, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
    use super::{MARK_END, MARK_START};
    let parts: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let Some(first) = parts.first() else {
        return Ok(Vec::new());
    };
    let mut stmt = conn.prepare_cached(
        "SELECT id, rel_path, title, plaintext FROM notes WHERE instr(lower(plaintext), ?1) > 0",
    )?;
    let rows = stmt.query_map([first], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))
    })?;
    let mut hits = Vec::new();
    for row in rows {
        let (note_id, rel_path, title, text) = row?;
        let lower = text.to_lowercase();
        if !parts.iter().all(|p| lower.contains(p.as_str())) {
            continue;
        }
        let count: usize = parts.iter().map(|p| lower.matches(p.as_str()).count()).sum();
        // Lower-casing can change byte lengths, so place the snippet by
        // characters in the lower-cased text.
        let at = lower.find(first.as_str()).unwrap_or(0);
        let chars: Vec<char> = lower.chars().collect();
        let at_char = lower[..at].chars().count();
        let len = first.chars().count();
        let from = at_char.saturating_sub(30);
        let to = (at_char + len + 30).min(chars.len());
        let snippet = format!(
            "{}{}{MARK_START}{}{MARK_END}{}{}",
            if from > 0 { "…" } else { "" },
            chars[from..at_char].iter().collect::<String>(),
            chars[at_char..at_char + len].iter().collect::<String>(),
            chars[at_char + len..to].iter().collect::<String>(),
            if to < chars.len() { "…" } else { "" },
        );
        hits.push(SearchHit {
            note_id,
            rel_path,
            title,
            snippet,
            score: count as f64,
            source: HitSource::FullText,
        });
    }
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    hits.truncate(limit);
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
    fn question_finds_notes_with_some_of_the_words() {
        let (dir, conn) = indexed_vault();
        fs::write(dir.path().join("backup.md"), "# Ops\n\nThe nightly backup job runs at 2am.").unwrap();
        index::reindex_all(&conn, dir.path()).unwrap();
        let hits = search_fulltext(&conn, "when does the backup run", 10).unwrap();
        assert_eq!(hits.first().map(|h| h.rel_path.as_str()), Some("backup.md"));
        let hits = search_fulltext(&conn, "nightly backup schedule", 10).unwrap();
        assert_eq!(hits.first().map(|h| h.rel_path.as_str()), Some("backup.md"));
    }

    #[test]
    fn finds_words_inside_japanese_sentences() {
        let (dir, conn) = indexed_vault();
        fs::write(dir.path().join("tokyo.md"), "# 旅行\n\n昨日は東京タワーに行った。").unwrap();
        index::reindex_all(&conn, dir.path()).unwrap();
        for query in ["東京", "タワー"] {
            let hits = search_fulltext(&conn, query, 10).unwrap();
            assert_eq!(hits.len(), 1, "{query}");
            assert!(hits[0].snippet.contains(&format!("{}{query}", super::super::MARK_START)));
        }
        assert!(search_fulltext(&conn, "議事録", 10).unwrap().is_empty());
    }

    #[test]
    fn phrase_search_needs_the_whole_phrase() {
        let (_d, conn) = indexed_vault();
        assert_eq!(search_phrase(&conn, "brown fox", 10).unwrap().len(), 1);
        assert!(search_phrase(&conn, "fox brown", 10).unwrap().is_empty());
        assert!(search_phrase(&conn, "brow", 10).unwrap().is_empty(), "no prefix matching");
    }

    #[test]
    fn empty_query_returns_nothing() {
        let (_d, conn) = indexed_vault();
        assert!(search_fulltext(&conn, "   ", 10).unwrap().is_empty());
    }
}
