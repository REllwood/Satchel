//! Indexer: sync the vault into the SQLite index incrementally, and resolve
//! `[[wikilinks]]` to note ids. Text/link/tag indexing lives here; chunk
//! embeddings are layered on in the search module (Phase C).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use crate::model::LinkKind;
use crate::parse;
use crate::vault::{self, ScannedNote};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IndexStats {
    pub added: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub removed: usize,
    pub total: usize,
}

fn stem_lower(rel_path: &str) -> String {
    let file = rel_path.rsplit('/').next().unwrap_or(rel_path);
    let stem = file.strip_suffix(".md").unwrap_or(file);
    stem.to_lowercase()
}

fn kind_str(kind: LinkKind) -> &'static str {
    match kind {
        LinkKind::Wikilink => "wikilink",
        LinkKind::Embed => "embed",
    }
}

/// Index a single scanned note. Returns its note id. Unchanged notes (same
/// hash) are skipped cheaply.
pub fn index_note(conn: &Connection, note: &ScannedNote) -> Result<i64> {
    let existing: Option<(i64, String, bool)> = conn
        .query_row(
            "SELECT id, hash, materialized FROM notes WHERE rel_path = ?1",
            [&note.rel_path],
            |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0)),
        )
        .optional()?;

    // Non-materialized (cloud-evicted) notes: record a stub with an EMPTY hash,
    // so that when the file later re-materializes with identical bytes it is
    // still detected as changed and re-indexed (otherwise the body is lost).
    if !note.materialized {
        let title = stem_lower(&note.rel_path);
        let id = upsert_note_row(conn, note, &title, "null", "", "", "")?;
        conn.execute("DELETE FROM links WHERE src_note_id = ?1", [id])?;
        conn.execute("DELETE FROM tags WHERE note_id = ?1", [id])?;
        conn.execute("DELETE FROM attachments WHERE note_id = ?1", [id])?;
        return Ok(id);
    }

    let bytes = std::fs::read(&note.abs_path)?;
    let hash = vault::content_hash(&bytes);
    if let Some((id, old_hash, old_materialized)) = &existing {
        // Skip only if unchanged AND already materialized (a stub has body="").
        if *old_hash == hash && *old_materialized {
            return Ok(*id);
        }
    }

    let content = String::from_utf8_lossy(&bytes);
    let parsed = parse::parse(&content);
    let title = if parsed.title.is_empty() {
        let file = note.rel_path.rsplit('/').next().unwrap_or(&note.rel_path);
        file.strip_suffix(".md").unwrap_or(file).to_string()
    } else {
        parsed.title.clone()
    };
    let fm = serde_json::to_string(&parsed.frontmatter).unwrap_or_else(|_| "null".into());
    // Fold frontmatter scalar values into the FTS plaintext so they're searchable too.
    let mut plaintext = parsed.plaintext.clone();
    append_frontmatter_text(&parsed.frontmatter, &mut plaintext);

    let id = upsert_note_row(conn, note, &title, &fm, &parsed.body, &plaintext, &hash)?;

    // Replace links / tags / attachments for this note.
    conn.execute("DELETE FROM links WHERE src_note_id = ?1", [id])?;
    conn.execute("DELETE FROM tags WHERE note_id = ?1", [id])?;
    conn.execute("DELETE FROM attachments WHERE note_id = ?1", [id])?;

    for link in &parsed.links {
        conn.execute(
            "INSERT INTO links (src_note_id, target, heading, alias, kind) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, link.target, link.heading, link.alias, kind_str(link.kind)],
        )?;
        // An embed of a non-markdown file is an attachment reference.
        if link.kind == LinkKind::Embed && !link.target.to_lowercase().ends_with(".md") {
            conn.execute(
                "INSERT INTO attachments (note_id, rel_path, mime) VALUES (?1, ?2, NULL)",
                params![id, link.target],
            )?;
        }
    }
    for tag in &parsed.tags {
        conn.execute(
            "INSERT OR IGNORE INTO tags (note_id, tag) VALUES (?1, ?2)",
            params![id, tag],
        )?;
    }

    Ok(id)
}

#[allow(clippy::too_many_arguments)]
fn upsert_note_row(
    conn: &Connection,
    note: &ScannedNote,
    title: &str,
    frontmatter_json: &str,
    body: &str,
    plaintext: &str,
    hash: &str,
) -> Result<i64> {
    let id: i64 = conn.query_row(
        "INSERT INTO notes (rel_path, title, frontmatter, body, plaintext, mtime, size, hash, materialized)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(rel_path) DO UPDATE SET
            title=excluded.title, frontmatter=excluded.frontmatter, body=excluded.body,
            plaintext=excluded.plaintext, mtime=excluded.mtime, size=excluded.size,
            hash=excluded.hash, materialized=excluded.materialized
         RETURNING id",
        params![
            note.rel_path,
            title,
            frontmatter_json,
            body,
            plaintext,
            note.mtime_ms,
            note.size as i64,
            hash,
            note.materialized as i64,
        ],
        |r| r.get(0),
    )?;
    Ok(id)
}

/// Append frontmatter keys + scalar values into the FTS plaintext projection so
/// they surface in full-text search (not just via the query view).
fn append_frontmatter_text(fm: &serde_json::Value, out: &mut String) {
    use serde_json::Value;
    match fm {
        Value::String(s) => {
            out.push(' ');
            out.push_str(s);
        }
        Value::Number(n) => {
            out.push(' ');
            out.push_str(&n.to_string());
        }
        Value::Bool(b) => {
            out.push(' ');
            out.push_str(if *b { "true" } else { "false" });
        }
        Value::Array(items) => items.iter().for_each(|v| append_frontmatter_text(v, out)),
        Value::Object(map) => map.iter().for_each(|(k, v)| {
            out.push(' ');
            out.push_str(k);
            append_frontmatter_text(v, out);
        }),
        Value::Null => {}
    }
}

/// Look up a note id by its relative path.
pub fn note_id_by_path(conn: &Connection, rel_path: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row("SELECT id FROM notes WHERE rel_path = ?1", [rel_path], |r| {
            r.get(0)
        })
        .optional()?)
}

/// Remove a note (and, via FK cascade, its links/tags/attachments/chunks).
/// Also clears its vectors, which aren't FK-linked to `notes`.
pub fn remove_note(conn: &Connection, rel_path: &str) -> Result<()> {
    if let Some(id) = note_id_by_path(conn, rel_path)? {
        crate::db::vec::delete_note_vectors(conn, id)?;
    }
    conn.execute("DELETE FROM notes WHERE rel_path = ?1", [rel_path])?;
    Ok(())
}

/// Resolve every link's `target` to a note id where possible. Priority:
/// exact relative path → filename stem → title (all case-insensitive).
pub fn resolve_links(conn: &Connection) -> Result<()> {
    let mut by_relpath: HashMap<String, i64> = HashMap::new();
    let mut by_stem: HashMap<String, i64> = HashMap::new();
    let mut by_title: HashMap<String, i64> = HashMap::new();

    {
        let mut stmt = conn.prepare("SELECT id, rel_path, title FROM notes")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (id, rel_path, title) = row?;
            let rp = rel_path.to_lowercase();
            by_relpath.insert(rp.clone(), id);
            by_relpath
                .entry(rp.strip_suffix(".md").unwrap_or(&rp).to_string())
                .or_insert(id);
            // First note wins for an ambiguous stem (deterministic via id order).
            by_stem.entry(stem_lower(&rel_path)).or_insert(id);
            if !title.is_empty() {
                by_title.entry(title.to_lowercase()).or_insert(id);
            }
        }
    }

    // Current state of every link, so we only WRITE rows whose resolution
    // actually changes (a single edit usually changes nothing — avoids a
    // table-wide UPDATE storm on the single writer connection).
    let links: Vec<(i64, String, Option<i64>, bool)> = {
        let mut stmt =
            conn.prepare("SELECT id, target, target_note_id, resolved FROM links")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, i64>(3)? != 0,
            ))
        })?;
        rows.collect::<std::result::Result<_, _>>()?
    };

    for (link_id, target, cur_target, cur_resolved) in links {
        let key = target.to_lowercase();
        let resolved = by_relpath
            .get(&key)
            .or_else(|| by_relpath.get(key.strip_suffix(".md").unwrap_or(&key)))
            .or_else(|| by_stem.get(&key))
            .or_else(|| by_title.get(&key))
            .copied();
        // Skip the write if nothing changed.
        if resolved == cur_target && resolved.is_some() == cur_resolved {
            continue;
        }
        match resolved {
            Some(target_id) => conn.execute(
                "UPDATE links SET target_note_id = ?2, resolved = 1 WHERE id = ?1",
                params![link_id, target_id],
            )?,
            None => conn.execute(
                "UPDATE links SET target_note_id = NULL, resolved = 0 WHERE id = ?1",
                [link_id],
            )?,
        };
    }
    Ok(())
}

/// Index (or remove, if gone) a single note by relative path, then re-resolve
/// links. Used after in-app edits and by the file watcher.
pub fn index_single(conn: &Connection, root: &Path, rel_path: &str) -> Result<Option<i64>> {
    let abs = root.join(rel_path);
    let id = if abs.is_file() {
        let meta = std::fs::metadata(&abs)?;
        let mtime_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let scanned = ScannedNote {
            rel_path: rel_path.to_string(),
            abs_path: abs,
            materialized: true,
            mtime_ms,
            size: meta.len(),
        };
        Some(index_note(conn, &scanned)?)
    } else {
        remove_note(conn, rel_path)?;
        None
    };
    resolve_links(conn)?;
    Ok(id)
}

/// Full reconcile: index every note in the vault, drop notes that vanished,
/// then resolve links.
pub fn reindex_all(conn: &Connection, root: &Path) -> Result<IndexStats> {
    let scanned = vault::scan_vault(root);
    let mut stats = IndexStats {
        total: scanned.len(),
        ..Default::default()
    };

    let on_disk: HashSet<String> = scanned.iter().map(|n| n.rel_path.clone()).collect();

    for note in &scanned {
        let prior: Option<String> = conn
            .query_row(
                "SELECT hash FROM notes WHERE rel_path = ?1",
                [&note.rel_path],
                |r| r.get(0),
            )
            .optional()?;
        let before_hash = prior.clone();
        index_note(conn, note)?;
        match before_hash {
            None => stats.added += 1,
            Some(h) => {
                let after: String = conn
                    .query_row(
                        "SELECT hash FROM notes WHERE rel_path = ?1",
                        [&note.rel_path],
                        |r| r.get(0),
                    )
                    .unwrap_or_default();
                if after == h {
                    stats.unchanged += 1;
                } else {
                    stats.updated += 1;
                }
            }
        }
    }

    // Drop notes whose files are gone.
    let existing: Vec<String> = {
        let mut stmt = conn.prepare("SELECT rel_path FROM notes")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    for rel in existing {
        if !on_disk.contains(&rel) {
            remove_note(conn, &rel)?;
            stats.removed += 1;
        }
    }

    resolve_links(conn)?;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use std::fs;

    fn fixture() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(
            root.join("alpha.md"),
            "---\ntags: [t1]\n---\n# Alpha\n\nlinks to [[Beta]] and #inline.",
        )
        .unwrap();
        fs::write(root.join("beta.md"), "# Beta\n\nno links here").unwrap();
        let conn = db::open_in_memory().unwrap();
        (dir, conn)
    }

    #[test]
    fn reindex_counts_and_resolves_links() {
        let (dir, conn) = fixture();
        let stats = reindex_all(&conn, dir.path()).unwrap();
        assert_eq!(stats.total, 2);
        assert_eq!(stats.added, 2);

        let notes: i64 = conn
            .query_row("SELECT count(*) FROM notes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(notes, 2);

        // The [[Beta]] link resolved to beta.md.
        let resolved: i64 = conn
            .query_row(
                "SELECT l.resolved FROM links l JOIN notes n ON n.id = l.target_note_id WHERE n.rel_path='beta.md'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(resolved, 1);

        let tags: i64 = conn
            .query_row("SELECT count(*) FROM tags WHERE tag='t1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(tags, 1);
    }

    #[test]
    fn incremental_update_only_changes_edited_note() {
        let (dir, conn) = fixture();
        reindex_all(&conn, dir.path()).unwrap();

        fs::write(dir.path().join("alpha.md"), "# Alpha v2\n\nchanged body").unwrap();
        let stats = reindex_all(&conn, dir.path()).unwrap();
        assert_eq!(stats.updated, 1);
        assert_eq!(stats.unchanged, 1);

        let title: String = conn
            .query_row(
                "SELECT title FROM notes WHERE rel_path='alpha.md'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(title, "Alpha v2");
    }

    #[test]
    fn deleting_a_file_removes_it_from_index() {
        let (dir, conn) = fixture();
        reindex_all(&conn, dir.path()).unwrap();
        fs::remove_file(dir.path().join("beta.md")).unwrap();

        let stats = reindex_all(&conn, dir.path()).unwrap();
        assert_eq!(stats.removed, 1);
        let notes: i64 = conn
            .query_row("SELECT count(*) FROM notes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(notes, 1);

        // Beta's backlink is now unresolved.
        let resolved: i64 = conn
            .query_row(
                "SELECT resolved FROM links WHERE target='Beta'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(resolved, 0);
    }

    #[test]
    fn rematerialized_file_with_same_bytes_is_reindexed() {
        // Regression: a cloud file evicted then re-downloaded with identical
        // bytes must not stay stuck as an empty stub.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("n.md"), "# N\n\nunique-token-xyz").unwrap();
        let conn = db::open_in_memory().unwrap();
        reindex_all(&conn, root).unwrap();
        let id = note_id_by_path(&conn, "n.md").unwrap().unwrap();

        // Evict → stub (non-materialized): body cleared.
        let stub = ScannedNote {
            rel_path: "n.md".into(),
            abs_path: root.join("n.md"),
            materialized: false,
            mtime_ms: 0,
            size: 0,
        };
        index_note(&conn, &stub).unwrap();
        let body: String = conn
            .query_row("SELECT body FROM notes WHERE id=?1", [id], |r| r.get(0))
            .unwrap();
        assert_eq!(body, "", "stub clears the body");

        // Re-materialize with the SAME bytes → must restore body + materialized.
        let scanned = vault::scan_vault(root)
            .into_iter()
            .find(|n| n.rel_path == "n.md")
            .unwrap();
        index_note(&conn, &scanned).unwrap();
        let (body, mat): (String, i64) = conn
            .query_row(
                "SELECT body, materialized FROM notes WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!(body.contains("unique-token-xyz"), "body restored on re-materialize");
        assert_eq!(mat, 1, "materialized flag restored");
    }

    #[test]
    fn frontmatter_values_are_full_text_searchable() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("a.md"),
            "---\nauthor: Hemingway\n---\n# A\n\nbody",
        )
        .unwrap();
        let conn = db::open_in_memory().unwrap();
        reindex_all(&conn, dir.path()).unwrap();
        let hits: i64 = conn
            .query_row(
                "SELECT count(*) FROM note_fts WHERE note_fts MATCH 'Hemingway'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 1, "frontmatter value should be searchable");
    }
}
