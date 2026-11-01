//! Indexer: sync the vault into the SQLite index incrementally, and resolve
//! `[[wikilinks]]` to note ids. Text/link/tag indexing lives here; chunk
//! embeddings are layered on in the search module.

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
    /// Notes found at a new path (moved or renamed outside the app). They keep
    /// their index row, so their content and embeddings carry over.
    pub moved: usize,
    /// Notes that could not be read this time. Their previous index entry is kept.
    pub skipped: usize,
    pub total: usize,
}

/// Lower-case and Unicode-normalize (NFC) a name, so `Café` typed on a
/// keyboard matches `Café.md` stored by macOS in decomposed form.
fn name_key(s: &str) -> String {
    let lower = s.to_lowercase();
    icu_normalizer::ComposingNormalizerBorrowed::new_nfc()
        .normalize(&lower)
        .into_owned()
}

/// Filename without folder or `.md`, case preserved.
fn file_stem(rel_path: &str) -> String {
    let file = rel_path.rsplit('/').next().unwrap_or(rel_path);
    file.strip_suffix(".md").unwrap_or(file).to_string()
}

fn stem_key(rel_path: &str) -> String {
    name_key(&file_stem(rel_path))
}

fn kind_str(kind: LinkKind) -> &'static str {
    match kind {
        LinkKind::Wikilink => "wikilink",
        LinkKind::Embed => "embed",
    }
}

/// Index a single scanned note. Returns its note id.
///
/// Cheap paths, in order:
/// 1. unchanged since last index (same mtime + size) → nothing is read, so a
///    cloud-evicted file is never downloaded just to be re-checked;
/// 2. contents only in the cloud → keep what we already indexed (still
///    searchable offline), or record a lightweight stub for a new note;
/// 3. read + hash; identical content just refreshes the stat info.
pub fn index_note(conn: &Connection, note: &ScannedNote) -> Result<i64> {
    Ok(index_note_changed(conn, note)?.0)
}

/// [`index_note`], also reporting whether the index changed (a new row, or new
/// content), so callers can skip re-resolving links when nothing did.
fn index_note_changed(conn: &Connection, note: &ScannedNote) -> Result<(i64, bool)> {
    // (id, hash, materialized, mtime, size)
    let existing: Option<(i64, String, bool, i64, i64)> = conn
        .query_row(
            "SELECT id, hash, materialized, mtime, size FROM notes WHERE rel_path = ?1",
            [&note.rel_path],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get::<_, i64>(2)? != 0,
                    r.get(3)?,
                    r.get(4)?,
                ))
            },
        )
        .optional()?;

    if let Some((id, _, true, mtime, size)) = &existing {
        if *mtime == note.mtime_ms && *size == note.size as i64 {
            return Ok((*id, false));
        }
    }

    if !note.materialized {
        if let Some((id, ..)) = &existing {
            return Ok((*id, false));
        }
        // Stub with an EMPTY hash: when the file is later downloaded it is
        // always detected as changed and fully indexed.
        let title = file_stem(&note.rel_path);
        let id = upsert_note_row(conn, note, &title, "null", "", "", "")?;
        conn.execute("DELETE FROM links WHERE src_note_id = ?1", [id])?;
        conn.execute("DELETE FROM tags WHERE note_id = ?1", [id])?;
        conn.execute("DELETE FROM attachments WHERE note_id = ?1", [id])?;
        return Ok((id, true));
    }

    let bytes = std::fs::read(&note.abs_path)?;
    let hash = vault::content_hash(&bytes);
    if let Some((id, old_hash, true, _, _)) = &existing {
        if *old_hash == hash {
            // Touched or re-synced without a content change: refresh the stat
            // info so the fast path hits next time.
            conn.execute(
                "UPDATE notes SET mtime = ?2, size = ?3 WHERE id = ?1",
                params![id, note.mtime_ms, note.size as i64],
            )?;
            return Ok((*id, false));
        }
    }

    let content = String::from_utf8_lossy(&bytes);
    let parsed = parse::parse(&content);
    let title = if parsed.title.is_empty() {
        file_stem(&note.rel_path)
    } else {
        parsed.title.clone()
    };
    let fm = serde_json::to_string(&parsed.frontmatter).unwrap_or_else(|_| "null".into());
    // Fold frontmatter scalar values into the FTS plaintext so they're searchable too.
    let mut plaintext = parsed.plaintext.clone();
    append_frontmatter_text(&parsed.frontmatter, &mut plaintext);

    let id = upsert_note_row(conn, note, &title, &fm, &parsed.body, &plaintext, &hash)?;

    // Content changed: drop stale embeddings so they get rebuilt (by the
    // background embedder, or right away by a caller that has one loaded).
    crate::db::vec::delete_note_vectors(conn, id)?;
    conn.execute("DELETE FROM chunks WHERE note_id = ?1", [id])?;

    // Replace links / tags / attachments for this note.
    conn.execute("DELETE FROM links WHERE src_note_id = ?1", [id])?;
    conn.execute("DELETE FROM tags WHERE note_id = ?1", [id])?;
    conn.execute("DELETE FROM attachments WHERE note_id = ?1", [id])?;

    for link in &parsed.links {
        conn.execute(
            "INSERT INTO links (src_note_id, target, heading, alias, kind) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, link.target, link.heading, link.alias, kind_str(link.kind)],
        )?;
        // An embed of a file other than a note (`![[pic.png]]`, not
        // `![[Other note]]`) is an attachment reference.
        if link.kind == LinkKind::Embed && is_attachment_target(&link.target) {
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

    Ok((id, true))
}

/// True for a link target naming a file with an extension other than `.md`.
fn is_attachment_target(target: &str) -> bool {
    let name = target.rsplit('/').next().unwrap_or(target);
    match name.rsplit_once('.') {
        Some((stem, ext)) => !stem.is_empty() && !ext.eq_ignore_ascii_case("md") && !ext.contains(' '),
        None => false,
    }
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
    remove_note_changed(conn, rel_path)?;
    Ok(())
}

fn remove_note_changed(conn: &Connection, rel_path: &str) -> Result<bool> {
    let Some(id) = note_id_by_path(conn, rel_path)? else {
        return Ok(false);
    };
    crate::db::vec::delete_note_vectors(conn, id)?;
    conn.execute("DELETE FROM notes WHERE id = ?1", [id])?;
    Ok(true)
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
            let rp = name_key(&rel_path);
            by_relpath.insert(rp.clone(), id);
            by_relpath
                .entry(rp.strip_suffix(".md").unwrap_or(&rp).to_string())
                .or_insert(id);
            // First note wins for an ambiguous stem (deterministic via id order).
            by_stem.entry(stem_key(&rel_path)).or_insert(id);
            if !title.is_empty() {
                by_title.entry(name_key(&title)).or_insert(id);
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
        let key = name_key(&target);
        let bare = key.strip_suffix(".md").unwrap_or(&key);
        let resolved = by_relpath
            .get(&key)
            .or_else(|| by_relpath.get(bare))
            .or_else(|| by_stem.get(&key))
            .or_else(|| by_stem.get(bare))
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

/// Index (or remove, if gone) one note without re-resolving links. Returns
/// its id (`None` if it is gone or is not a note) and whether the index
/// changed. Paths that are not notes, such as canvases or files in hidden
/// folders, are ignored.
fn index_path(conn: &Connection, root: &Path, rel_path: &str) -> Result<(Option<i64>, bool)> {
    if !vault::is_markdown(rel_path) || vault::is_hidden_path(rel_path) {
        return Ok((None, false));
    }
    let abs = root.join(rel_path);
    if !vault::exists_exactly(&abs) {
        // A macOS 13 iCloud placeholder stands in for an evicted note: keep
        // what was indexed, like a full scan does.
        if let Some(placeholder) = vault::icloud_placeholder(&abs) {
            let meta = std::fs::metadata(&placeholder)?;
            let scanned = ScannedNote {
                rel_path: rel_path.to_string(),
                abs_path: abs,
                materialized: false,
                mtime_ms: vault::mtime_ms(&meta),
                size: meta.len(),
            };
            let (id, changed) = index_note_changed(conn, &scanned)?;
            return Ok((Some(id), changed));
        }
        return Ok((None, remove_note_changed(conn, rel_path)?));
    }
    let meta = std::fs::metadata(&abs)?;
    let scanned = ScannedNote {
        rel_path: rel_path.to_string(),
        abs_path: abs,
        materialized: !vault::is_dataless(&meta),
        mtime_ms: vault::mtime_ms(&meta),
        size: meta.len(),
    };
    let (id, changed) = index_note_changed(conn, &scanned)?;
    Ok((Some(id), changed))
}

/// Index (or remove, if gone) a single note by relative path, then re-resolve
/// links if anything changed. Used after in-app edits. Returns `None` for a
/// path that is gone or is not a note.
pub fn index_single(conn: &Connection, root: &Path, rel_path: &str) -> Result<Option<i64>> {
    let (id, changed) = index_path(conn, root, rel_path)?;
    if changed {
        resolve_links(conn)?;
    }
    Ok(id)
}

/// Index a batch of changed paths, such as one file watcher report, then
/// re-resolve links once if anything changed. A path that fails to index (for
/// example, a file removed mid-read) is skipped so it cannot hold up the rest.
pub fn index_paths<S: AsRef<str>>(conn: &Connection, root: &Path, rel_paths: &[S]) -> Result<()> {
    let mut changed = false;
    for rel_path in rel_paths {
        if let Ok((_, c)) = index_path(conn, root, rel_path.as_ref()) {
            changed |= c;
        }
    }
    if changed {
        resolve_links(conn)?;
    }
    Ok(())
}

/// Full reconcile: index every note in the vault, drop notes that vanished,
/// then resolve links. Fails if the vault folder itself is missing or
/// unreadable, rather than treating it as empty and wiping the index.
pub fn reindex_all(conn: &Connection, root: &Path) -> Result<IndexStats> {
    let scan = vault::scan_vault_report(root)
        .map_err(|e| anyhow::anyhow!("cannot read the notes folder {}: {e}", root.display()))?;
    apply_scan(conn, &scan)
}

/// Bring the index in line with a scan made by [`vault::scan_vault_report`].
/// The scan is the slow part on a cloud drive and needs no database, so it
/// can run before taking a lock on the connection.
///
/// A note that cannot be read is skipped (its previous entry is kept), notes
/// in folders that could not be listed are not treated as deleted, and a note
/// that turns up at a new path keeps its row, so its content and embeddings
/// survive a move even when the file is only in the cloud.
pub fn apply_scan(conn: &Connection, scan: &vault::VaultScan) -> Result<IndexStats> {
    let scanned = &scan.notes;
    let mut stats = IndexStats {
        total: scanned.len(),
        ..Default::default()
    };
    let on_disk: HashSet<&str> = scanned.iter().map(|n| n.rel_path.as_str()).collect();
    let in_unreadable = |rel: &str| {
        scan.unreadable
            .iter()
            .any(|dir| rel.starts_with(dir.as_str()) && rel[dir.len()..].starts_with('/'))
    };

    // (rel_path, id, hash, mtime, size) of every indexed note.
    let indexed: Vec<(String, i64, String, i64, i64)> = {
        let mut stmt = conn.prepare("SELECT rel_path, id, hash, mtime, size FROM notes")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    let known: HashSet<&str> = indexed.iter().map(|(rel, ..)| rel.as_str()).collect();
    let mut vanished: Vec<&(String, i64, String, i64, i64)> = indexed
        .iter()
        .filter(|(rel, ..)| !on_disk.contains(rel.as_str()) && !in_unreadable(rel))
        .collect();

    // Follow notes that moved: same size and modification time as a vanished
    // note, and either the same file name (a folder move, which also works for
    // cloud-only files) or the same content.
    let mut changed = false;
    let mut moved: HashSet<&str> = HashSet::new();
    for note in scanned.iter().filter(|n| !known.contains(n.rel_path.as_str())) {
        let name = note.rel_path.rsplit('/').next().unwrap_or(&note.rel_path);
        let candidates: Vec<usize> = vanished
            .iter()
            .enumerate()
            .filter(|(_, (_, _, _, mtime, size))| *mtime == note.mtime_ms && *size == note.size as i64)
            .map(|(i, _)| i)
            .collect();
        let found = candidates.iter().copied().find(|&i| {
            let (rel, _, hash, ..) = vanished[i];
            rel.rsplit('/').next() == Some(name)
                || (candidates.len() == 1
                    && note.materialized
                    && std::fs::read(&note.abs_path).is_ok_and(|b| vault::content_hash(&b) == *hash))
        });
        if let Some(i) = found {
            let (_, id, ..) = vanished.remove(i);
            conn.execute("UPDATE notes SET rel_path = ?2 WHERE id = ?1", params![id, note.rel_path])?;
            moved.insert(note.rel_path.as_str());
            stats.moved += 1;
            changed = true;
        }
    }

    for note in scanned {
        let rel = note.rel_path.as_str();
        match index_note_changed(conn, note) {
            Ok((_, c)) => {
                changed |= c;
                if moved.contains(rel) {
                    // Counted as moved already.
                } else if !known.contains(rel) {
                    stats.added += 1;
                } else if c {
                    stats.updated += 1;
                } else {
                    stats.unchanged += 1;
                }
            }
            Err(_) => stats.skipped += 1,
        }
    }

    for (rel, ..) in vanished {
        remove_note(conn, rel)?;
        stats.removed += 1;
        changed = true;
    }

    if changed {
        resolve_links(conn)?;
    }
    Ok(stats)
}

/// A link that resolved to a note before the note moved.
#[derive(Debug, Clone)]
pub struct InboundLink {
    src_note_id: i64,
    target_note_id: i64,
    target: String,
    /// Where the target note was when the link was captured.
    target_rel_before: String,
}

/// Every link currently resolved to one of `note_ids`. Capture these before a
/// rename so [`rewrite_moved_links`] can fix the ones it breaks.
pub fn inbound_links(conn: &Connection, note_ids: &[i64]) -> Result<Vec<InboundLink>> {
    let mut stmt = conn.prepare_cached(
        "SELECT l.src_note_id, l.target_note_id, l.target, n.rel_path
         FROM links l JOIN notes n ON n.id = l.target_note_id
         WHERE l.target_note_id = ?1",
    )?;
    let mut out = Vec::new();
    for &id in note_ids {
        let rows = stmt.query_map([id], |r| {
            Ok(InboundLink {
                src_note_id: r.get(0)?,
                target_note_id: r.get(1)?,
                target: r.get(2)?,
                target_rel_before: r.get(3)?,
            })
        })?;
        for row in rows {
            out.push(row?);
        }
    }
    Ok(out)
}

/// Ids of the notes in folder `dir` (vault-relative), at any depth.
pub fn note_ids_in_folder(conn: &Connection, dir: &str) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM notes WHERE substr(rel_path, 1, length(?1) + 1) = ?1 || '/'",
    )?;
    let ids = stmt.query_map([dir], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
    Ok(ids)
}

/// Record that the note at `from` was renamed or moved to `to`. Its index row
/// (and so its id, content, embeddings and backlinks) moves with it, which
/// matters most for a note whose contents are only in the cloud. Returns the
/// note's id at the new path, if `to` is a note.
pub fn rename_note(conn: &Connection, root: &Path, from: &str, to: &str) -> Result<Option<i64>> {
    if from != to && vault::is_markdown(to) && !vault::is_hidden_path(to) {
        if let Some(id) = note_id_by_path(conn, from)? {
            // A case-only rename on a case-insensitive disk has no other row
            // at `to`. Any row that is there is stale.
            if note_id_by_path(conn, to)?.is_some_and(|other| other != id) {
                remove_note(conn, to)?;
            }
            conn.execute("UPDATE notes SET rel_path = ?2 WHERE id = ?1", params![id, to])?;
        }
    }
    index_path(conn, root, from)?;
    let (id, _) = index_path(conn, root, to)?;
    resolve_links(conn)?;
    Ok(id)
}

/// Record that folder `from` was renamed or moved to `to`: every note in it
/// keeps its index row under the new path.
pub fn rename_folder(conn: &Connection, from: &str, to: &str) -> Result<()> {
    conn.execute(
        "UPDATE notes SET rel_path = ?2 || substr(rel_path, length(?1) + 1)
         WHERE substr(rel_path, 1, length(?1) + 1) = ?1 || '/'",
        params![from, to],
    )?;
    resolve_links(conn)
}

/// Drop every note in folder `dir` from the index (after it was deleted).
pub fn remove_folder(conn: &Connection, dir: &str) -> Result<()> {
    for id in note_ids_in_folder(conn, dir)? {
        crate::db::vec::delete_note_vectors(conn, id)?;
        conn.execute("DELETE FROM notes WHERE id = ?1", [id])?;
    }
    resolve_links(conn)
}

/// After notes moved, rewrite links in other notes that named them by file
/// name or path, so `[[Old name]]` follows a rename to `[[New name]]`. Links
/// that still name the file (for example `[[Name]]` after a move to another
/// folder) and links that matched the note's title are left alone. Returns
/// the notes rewritten.
pub fn rewrite_moved_links(conn: &Connection, root: &Path, before: &[InboundLink]) -> Result<Vec<String>> {
    use std::collections::BTreeMap;
    // Source note id -> (old target text key -> new target text).
    let mut by_source: BTreeMap<i64, HashMap<String, String>> = BTreeMap::new();
    for link in before {
        let Some(rel_now) = conn
            .query_row("SELECT rel_path FROM notes WHERE id = ?1", [link.target_note_id], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
        else {
            continue;
        };
        if !names_file(&link.target, &link.target_rel_before) || names_file(&link.target, &rel_now) {
            continue;
        }
        let Some(new_target) = link_text_for(conn, link.target_note_id, &link.target)? else {
            continue;
        };
        by_source
            .entry(link.src_note_id)
            .or_default()
            .insert(name_key(&link.target), new_target);
    }

    let mut rewritten = Vec::new();
    for (src_id, targets) in by_source {
        let Some(src_rel) = conn
            .query_row("SELECT rel_path FROM notes WHERE id = ?1", [src_id], |r| r.get::<_, String>(0))
            .optional()?
        else {
            continue;
        };
        let Ok(abs) = vault::safe_join(root, &src_rel) else { continue };
        let Ok(content) = std::fs::read_to_string(&abs) else { continue };
        if let Some(updated) = rewrite_wikilinks(&content, &targets) {
            vault::write_note_atomic(&abs, &updated)?;
            index_single(conn, root, &src_rel)?;
            rewritten.push(src_rel);
        }
    }
    Ok(rewritten)
}

/// True if link text `target` names the note at `rel` by its path or file
/// name (with or without `.md`), rather than by its title.
fn names_file(target: &str, rel: &str) -> bool {
    let key = name_key(target);
    let key = key.strip_suffix(".md").unwrap_or(&key);
    let rel = name_key(rel);
    key == rel.strip_suffix(".md").unwrap_or(&rel) || key == stem_key(&rel)
}

/// How a link should name note `id` now, in the same style as `old` (a bare
/// name, or a path, with or without `.md`). A bare name that another note
/// shares becomes a path, so the link stays unambiguous.
fn link_text_for(conn: &Connection, id: i64, old: &str) -> Result<Option<String>> {
    let Some(rel) = conn
        .query_row("SELECT rel_path FROM notes WHERE id = ?1", [id], |r| r.get::<_, String>(0))
        .optional()?
    else {
        return Ok(None);
    };
    let with_md = old.to_lowercase().ends_with(".md");
    let stem = file_stem(&rel);
    let shared = {
        let mut stmt = conn.prepare_cached("SELECT rel_path FROM notes WHERE id != ?1")?;
        let others = stmt.query_map([id], |r| r.get::<_, String>(0))?;
        let key = stem_key(&rel);
        let mut shared = false;
        for other in others {
            shared |= stem_key(&other?) == key;
        }
        shared
    };
    let text = if old.contains('/') || shared {
        rel.strip_suffix(".md").unwrap_or(&rel).to_string()
    } else {
        stem
    };
    Ok(Some(if with_md { format!("{text}.md") } else { text }))
}

/// Replace the targets of `[[wikilinks]]` (and `![[embeds]]`) in `content`,
/// keeping headings, aliases and everything else. `targets` maps a
/// [`name_key`] of the old target text to the new text. Links inside code
/// are left alone. `None` if nothing changed.
fn rewrite_wikilinks(content: &str, targets: &HashMap<String, String>) -> Option<String> {
    let (_, body) = parse::split_frontmatter(content);
    let offset = content.len() - body.len();
    let code: Vec<std::ops::Range<usize>> = parse::code_ranges(&body)
        .into_iter()
        .map(|r| r.start + offset..r.end + offset)
        .collect();
    let mut out = String::with_capacity(content.len());
    let mut last = 0;
    for cap in parse::WIKILINK.captures_iter(&content[offset..]) {
        let inner = cap.get(2).unwrap();
        let range = inner.start() + offset..inner.end() + offset;
        if code.iter().any(|c| c.start <= range.start && range.end <= c.end) {
            continue;
        }
        let (target, heading, alias) = parse::split_wikilink(inner.as_str());
        let Some(new_target) = targets.get(&name_key(&target)) else {
            continue;
        };
        let alias_sep = if inner.as_str().contains("\\|") { "\\|" } else { "|" };
        let mut replacement = new_target.clone();
        if let Some(h) = heading {
            replacement.push('#');
            replacement.push_str(&h);
        }
        if let Some(a) = alias {
            replacement.push_str(alias_sep);
            replacement.push_str(&a);
        }
        out.push_str(&content[last..range.start]);
        out.push_str(&replacement);
        last = range.end;
    }
    if last == 0 {
        return None;
    }
    out.push_str(&content[last..]);
    Some(out)
}

/// Notes that mention note `id`'s title as a whole phrase without linking to
/// it, best first, as (rel_path, title). Titles with fewer than three letters
/// or digits are skipped, because they would match almost everything.
pub fn unlinked_mentions(conn: &Connection, id: i64, limit: usize) -> Result<Vec<(String, String)>> {
    let Some(title) = conn
        .query_row("SELECT title FROM notes WHERE id = ?1", [id], |r| r.get::<_, String>(0))
        .optional()?
    else {
        return Ok(Vec::new());
    };
    if title.chars().filter(|c| c.is_alphanumeric()).count() < 3 {
        return Ok(Vec::new());
    }
    let needle = title.to_lowercase();
    let candidates: Vec<(i64, String, String)> = if crate::search::fts::has_cjk(&title) {
        let mut stmt = conn.prepare("SELECT id, rel_path, title FROM notes WHERE instr(lower(plaintext), ?1) > 0")?;
        let rows = stmt.query_map([&needle], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<std::result::Result<_, _>>()?
    } else {
        crate::search::fts::search_phrase(conn, &title, 200)?
            .into_iter()
            .map(|h| (h.note_id, h.rel_path, h.title))
            .collect()
    };
    let mut out = Vec::new();
    for (note_id, rel_path, note_title) in candidates {
        if note_id == id || out.len() >= limit {
            continue;
        }
        let links_here: bool = conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM links WHERE src_note_id = ?1 AND target_note_id = ?2)",
            params![note_id, id],
            |r| r.get(0),
        )?;
        let text: String = conn.query_row("SELECT plaintext FROM notes WHERE id = ?1", [note_id], |r| r.get(0))?;
        if !links_here && contains_phrase(&text.to_lowercase(), &needle) {
            out.push((rel_path, note_title));
        }
    }
    Ok(out)
}

/// True if `needle` occurs in `haystack` as whole words (not inside a longer
/// word such as "plan" in "planet"). Both are lower-case.
fn contains_phrase(haystack: &str, needle: &str) -> bool {
    let is_word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric());
    haystack.match_indices(needle).any(|(at, m)| {
        let before = haystack[..at].chars().next_back();
        let after = haystack[at + m.len()..].chars().next();
        let first = needle.chars().next();
        let last = needle.chars().next_back();
        let starts_cleanly = !is_word(first) || !is_word(before);
        let ends_cleanly = !is_word(last) || !is_word(after);
        (starts_cleanly && ends_cleanly) || crate::search::fts::has_cjk(needle)
    })
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

    fn note_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT count(*) FROM notes", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn canvas_files_are_never_indexed_as_notes() {
        let (dir, conn) = fixture();
        reindex_all(&conn, dir.path()).unwrap();
        fs::write(
            dir.path().join("board.canvas"),
            r#"{"nodes":[{"id":"a","type":"text","text":"hi","x":0,"y":0,"width":10,"height":10}],"edges":[]}"#,
        )
        .unwrap();

        assert_eq!(index_single(&conn, dir.path(), "board.canvas").unwrap(), None);
        index_paths(&conn, dir.path(), &["board.canvas"]).unwrap();
        assert_eq!(note_count(&conn), 2);
    }

    #[test]
    fn index_paths_applies_a_whole_batch() {
        let (dir, conn) = fixture();
        reindex_all(&conn, dir.path()).unwrap();

        // One sync batch: beta deleted, gamma added, alpha now links to gamma.
        fs::remove_file(dir.path().join("beta.md")).unwrap();
        fs::write(dir.path().join("gamma.md"), "# Gamma").unwrap();
        fs::write(dir.path().join("alpha.md"), "# Alpha\n\nsee [[Gamma]]").unwrap();
        index_paths(&conn, dir.path(), &["beta.md", "gamma.md", "alpha.md", "missing.md"])
            .unwrap();

        let paths: Vec<String> = conn
            .prepare("SELECT rel_path FROM notes ORDER BY rel_path")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert_eq!(paths, vec!["alpha.md", "gamma.md"]);
        let resolved: i64 = conn
            .query_row("SELECT resolved FROM links WHERE target = 'Gamma'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(resolved, 1, "links resolve against the whole batch");
    }

    #[test]
    fn missing_vault_folder_is_an_error_not_an_empty_vault() {
        let (dir, conn) = fixture();
        reindex_all(&conn, dir.path()).unwrap();
        let gone = dir.path().with_extension("unmounted");
        fs::rename(dir.path(), &gone).unwrap();
        assert!(reindex_all(&conn, dir.path()).is_err());
        assert_eq!(note_count(&conn), 2, "the index is untouched");
        fs::rename(&gone, dir.path()).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_files_and_folders_do_not_drop_notes() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, conn) = fixture();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub/c.md"), "# C").unwrap();
        reindex_all(&conn, dir.path()).unwrap();

        let locked = |path: &Path, mode| fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
        locked(&dir.path().join("sub"), 0o000);
        fs::write(dir.path().join("new.md"), "# New").unwrap();
        locked(&dir.path().join("new.md"), 0o000);
        fs::write(dir.path().join("alpha.md"), "# Alpha v2").unwrap();
        let stats = reindex_all(&conn, dir.path());
        locked(&dir.path().join("sub"), 0o755);
        locked(&dir.path().join("new.md"), 0o644);

        let stats = stats.unwrap();
        assert_eq!(stats.removed, 0, "notes in an unreadable folder are not deleted");
        assert_eq!(stats.skipped, 1, "the unreadable note is skipped, not fatal");
        assert_eq!(stats.updated, 1, "the rest of the vault is still indexed");
        assert!(note_id_by_path(&conn, "sub/c.md").unwrap().is_some());
    }

    #[test]
    fn moved_notes_keep_their_index_row() {
        let (dir, conn) = fixture();
        reindex_all(&conn, dir.path()).unwrap();
        let beta = note_id_by_path(&conn, "beta.md").unwrap().unwrap();
        let alpha = note_id_by_path(&conn, "alpha.md").unwrap().unwrap();

        fs::create_dir(dir.path().join("Archive")).unwrap();
        fs::rename(dir.path().join("beta.md"), dir.path().join("Archive/beta.md")).unwrap();
        fs::rename(dir.path().join("alpha.md"), dir.path().join("first.md")).unwrap();
        let stats = reindex_all(&conn, dir.path()).unwrap();

        assert_eq!((stats.moved, stats.added, stats.removed), (2, 0, 0));
        assert_eq!(note_id_by_path(&conn, "Archive/beta.md").unwrap(), Some(beta));
        assert_eq!(note_id_by_path(&conn, "first.md").unwrap(), Some(alpha));
    }

    #[test]
    fn hidden_and_non_note_paths_are_never_indexed() {
        let (dir, conn) = fixture();
        fs::write(dir.path().join(".env.md"), "# secret").unwrap();
        fs::create_dir(dir.path().join(".obsidian")).unwrap();
        fs::write(dir.path().join(".obsidian/x.md"), "# x").unwrap();
        assert_eq!(index_single(&conn, dir.path(), ".env.md").unwrap(), None);
        assert_eq!(index_single(&conn, dir.path(), ".obsidian/x.md").unwrap(), None);
        assert_eq!(note_count(&conn), 0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn case_only_rename_does_not_duplicate_the_note() {
        let (dir, conn) = fixture();
        reindex_all(&conn, dir.path()).unwrap();
        fs::rename(dir.path().join("beta.md"), dir.path().join("Beta.md")).unwrap();
        // The watcher reports both spellings, and the old one still "exists"
        // on a case-insensitive disk.
        index_paths(&conn, dir.path(), &["beta.md", "Beta.md"]).unwrap();
        assert!(note_id_by_path(&conn, "beta.md").unwrap().is_none());
        assert!(note_id_by_path(&conn, "Beta.md").unwrap().is_some());
        assert_eq!(note_count(&conn), 2);
    }

    #[test]
    fn links_resolve_across_unicode_forms_and_with_md() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        // "Café" with a decomposed é, as macOS often stores names.
        fs::write(dir.path().join("Cafe\u{301}.md"), "# Menu").unwrap();
        fs::write(dir.path().join("sub/Deep.md"), "# Deep").unwrap();
        fs::write(
            dir.path().join("a.md"),
            "[[Caf\u{e9}]] [[Deep.md]] | [[Target Note\\|shown]] | ![[Deep]]",
        )
        .unwrap();
        fs::write(dir.path().join("Target Note.md"), "# T").unwrap();
        let conn = db::open_in_memory().unwrap();
        reindex_all(&conn, dir.path()).unwrap();
        let unresolved: Vec<String> = conn
            .prepare("SELECT target FROM links WHERE resolved = 0")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert!(unresolved.is_empty(), "unresolved: {unresolved:?}");
        let attachments: i64 = conn
            .query_row("SELECT count(*) FROM attachments", [], |r| r.get(0))
            .unwrap();
        assert_eq!(attachments, 0, "embedding a note is not an attachment");
    }

    #[test]
    fn renaming_a_note_keeps_its_row_and_rewrites_links_to_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("Old.md"), "# Old").unwrap();
        fs::write(
            root.join("a.md"),
            "See [[Old]], [[old#Intro|the intro]] and ![[Old]].\n\n`[[Old]]` in code stays.\n",
        )
        .unwrap();
        let conn = db::open_in_memory().unwrap();
        reindex_all(&conn, root).unwrap();
        let id = note_id_by_path(&conn, "Old.md").unwrap().unwrap();

        let before = inbound_links(&conn, &[id]).unwrap();
        fs::rename(root.join("Old.md"), root.join("New.md")).unwrap();
        assert_eq!(rename_note(&conn, root, "Old.md", "New.md").unwrap(), Some(id));
        let rewritten = rewrite_moved_links(&conn, root, &before).unwrap();

        assert_eq!(rewritten, vec!["a.md".to_string()]);
        assert_eq!(
            fs::read_to_string(root.join("a.md")).unwrap(),
            "See [[New]], [[New#Intro|the intro]] and ![[New]].\n\n`[[Old]]` in code stays.\n"
        );
        let unresolved: i64 = conn
            .query_row("SELECT count(*) FROM links WHERE resolved = 0", [], |r| r.get(0))
            .unwrap();
        assert_eq!(unresolved, 0);
    }

    #[test]
    fn moving_a_folder_rewrites_only_path_style_links() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir(root.join("Proj")).unwrap();
        fs::write(root.join("Proj/Plan.md"), "# Plan").unwrap();
        fs::write(root.join("a.md"), "[[Plan]] and [[Proj/Plan]]").unwrap();
        let conn = db::open_in_memory().unwrap();
        reindex_all(&conn, root).unwrap();
        let ids = note_ids_in_folder(&conn, "Proj").unwrap();
        let before = inbound_links(&conn, &ids).unwrap();

        fs::rename(root.join("Proj"), root.join("Work")).unwrap();
        rename_folder(&conn, "Proj", "Work").unwrap();
        rewrite_moved_links(&conn, root, &before).unwrap();

        assert_eq!(fs::read_to_string(root.join("a.md")).unwrap(), "[[Plan]] and [[Work/Plan]]");
        assert_eq!(note_ids_in_folder(&conn, "Work").unwrap(), ids);
    }

    #[test]
    fn unlinked_mentions_match_whole_phrases_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("Project Plan.md"), "# Project Plan").unwrap();
        fs::write(root.join("C++.md"), "# C++").unwrap();
        fs::write(root.join("yes.md"), "The project plan is ready.").unwrap();
        fs::write(root.join("planet.md"), "A project about the planet.").unwrap();
        fs::write(root.join("linked.md"), "The project plan, see [[Project Plan]].").unwrap();
        fs::write(root.join("cooking.md"), "Cooking with c and more c.").unwrap();
        let conn = db::open_in_memory().unwrap();
        reindex_all(&conn, root).unwrap();

        let plan = note_id_by_path(&conn, "Project Plan.md").unwrap().unwrap();
        let mentions: Vec<String> = unlinked_mentions(&conn, plan, 50)
            .unwrap()
            .into_iter()
            .map(|(rel, _)| rel)
            .collect();
        assert_eq!(mentions, vec!["yes.md".to_string()]);
        let cpp = note_id_by_path(&conn, "C++.md").unwrap().unwrap();
        assert!(unlinked_mentions(&conn, cpp, 50).unwrap().is_empty());
    }

    fn body_of(conn: &Connection, id: i64) -> (String, i64) {
        conn.query_row(
            "SELECT body, materialized FROM notes WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    }

    fn scanned(root: &Path, rel: &str) -> ScannedNote {
        vault::scan_vault(root)
            .into_iter()
            .find(|n| n.rel_path == rel)
            .unwrap()
    }

    #[test]
    fn evicted_note_keeps_its_indexed_content() {
        // A note that iCloud/Google evicts to the cloud must stay searchable
        // and must not be downloaded just to re-index it.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("n.md"), "# N\n\nunique-token-xyz").unwrap();
        let conn = db::open_in_memory().unwrap();
        reindex_all(&conn, root).unwrap();
        let id = note_id_by_path(&conn, "n.md").unwrap().unwrap();

        let mut evicted = scanned(root, "n.md");
        evicted.materialized = false;
        index_note(&conn, &evicted).unwrap();
        let (body, _) = body_of(&conn, id);
        assert!(body.contains("unique-token-xyz"), "indexed content is kept");

        // Downloading it again (same bytes) keeps everything intact.
        index_note(&conn, &scanned(root, "n.md")).unwrap();
        let (body, mat) = body_of(&conn, id);
        assert!(body.contains("unique-token-xyz"));
        assert_eq!(mat, 1);
    }

    #[test]
    fn never_indexed_cloud_note_is_a_stub_until_downloaded() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("Cloud Note.md"), "# Cloud\n\nfetched-later").unwrap();
        let conn = db::open_in_memory().unwrap();

        let mut cloud_only = scanned(root, "Cloud Note.md");
        cloud_only.materialized = false;
        let id = index_note(&conn, &cloud_only).unwrap();
        let (body, mat) = body_of(&conn, id);
        assert_eq!((body.as_str(), mat), ("", 0), "stub: nothing read");
        let title: String = conn
            .query_row("SELECT title FROM notes WHERE id=?1", [id], |r| r.get(0))
            .unwrap();
        assert_eq!(title, "Cloud Note", "stub title keeps the filename's case");

        // Once downloaded (same mtime/size) it is fully indexed.
        index_note(&conn, &scanned(root, "Cloud Note.md")).unwrap();
        let (body, mat) = body_of(&conn, id);
        assert!(body.contains("fetched-later"));
        assert_eq!(mat, 1);
    }

    #[test]
    fn unchanged_file_is_not_reread() {
        // Proves the mtime+size fast path: corrupt the stored hash, and an
        // unchanged file still isn't re-read (hash stays corrupted).
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.md"), "# A\n\nbody").unwrap();
        let conn = db::open_in_memory().unwrap();
        reindex_all(&conn, root).unwrap();
        conn.execute("UPDATE notes SET hash = 'sentinel'", []).unwrap();
        reindex_all(&conn, root).unwrap();
        let hash: String = conn
            .query_row("SELECT hash FROM notes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(hash, "sentinel", "file was not re-read");
    }

    #[test]
    fn content_change_drops_stale_embedding_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.md"), "# A\n\noriginal").unwrap();
        let conn = db::open_in_memory().unwrap();
        reindex_all(&conn, root).unwrap();
        let id = note_id_by_path(&conn, "a.md").unwrap().unwrap();
        conn.execute("INSERT INTO chunks (note_id, text) VALUES (?1, 'old')", [id])
            .unwrap();

        fs::write(root.join("a.md"), "# A\n\nrewritten with more words").unwrap();
        reindex_all(&conn, root).unwrap();
        let chunks: i64 = conn
            .query_row("SELECT count(*) FROM chunks WHERE note_id=?1", [id], |r| r.get(0))
            .unwrap();
        assert_eq!(chunks, 0, "stale chunks removed so they get re-embedded");
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
