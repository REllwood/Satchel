//! Semantic search: embed note chunks into sqlite-vec and run KNN queries,
//! plus a hybrid merge with full-text results.

use std::collections::HashMap;

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use super::{fts, HitSource, SearchHit};
use crate::db::vec;
use crate::embed::{self, Embedder};

/// Ensure the vector table exists for the embedder's dimensionality.
pub fn ensure_semantic(conn: &Connection, dim: u32) -> Result<()> {
    vec::create_vec_table(conn, dim)
}

/// (Re)embed a single note's chunks into `chunks` + `vec_chunks`.
/// Returns the number of chunks embedded.
pub fn index_note_embeddings(conn: &Connection, embedder: &Embedder, note_id: i64) -> Result<usize> {
    let row: Option<(String, bool)> = conn
        .query_row(
            "SELECT body, materialized FROM notes WHERE id = ?1",
            [note_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? != 0)),
        )
        .optional()?;
    let Some((body, materialized)) = row else {
        return Ok(0);
    };

    // Clear any previous chunks/vectors for this note.
    vec::delete_note_vectors(conn, note_id)?;
    conn.execute("DELETE FROM chunks WHERE note_id = ?1", [note_id])?;

    if !materialized {
        return Ok(0);
    }
    let chunks = embed::chunk_text(&body);
    if chunks.is_empty() {
        return Ok(0);
    }

    let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
    let vectors = embedder.embed(&texts)?;

    for (chunk, vector) in chunks.iter().zip(vectors.iter()) {
        let chunk_id: i64 = conn.query_row(
            "INSERT INTO chunks (note_id, ord, start, \"end\", text) VALUES (?1, ?2, ?3, ?4, ?5) RETURNING id",
            params![note_id, chunk.ord as i64, chunk.start as i64, chunk.end as i64, chunk.text],
            |r| r.get(0),
        )?;
        conn.execute(
            "INSERT INTO vec_chunks (chunk_id, embedding) VALUES (?1, ?2)",
            params![chunk_id, vec::f32_blob(vector)],
        )?;
    }
    Ok(chunks.len())
}

/// Embed every materialized note. Returns total chunks embedded.
pub fn embed_all(conn: &Connection, embedder: &Embedder) -> Result<usize> {
    ensure_semantic(conn, embedder.dim() as u32)?;
    let ids: Vec<i64> = {
        let mut stmt = conn.prepare("SELECT id FROM notes WHERE materialized = 1")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    let mut total = 0;
    for id in ids {
        total += index_note_embeddings(conn, embedder, id)?;
    }
    Ok(total)
}

/// Cosine similarity from a sqlite-vec L2 distance over normalized vectors.
fn score_from_distance(distance: f64) -> f64 {
    1.0 - (distance * distance) / 2.0
}

/// Semantic search: embed the query, KNN over chunk vectors, aggregate to the
/// best-matching note (min distance), and return ranked hits.
pub fn search_semantic(
    conn: &Connection,
    embedder: &Embedder,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let qvec = embedder.embed_one(query)?;
    let k = (limit * 4).max(10) as i64;

    let mut stmt = conn.prepare(
        "WITH knn AS (
            SELECT chunk_id, distance FROM vec_chunks
            WHERE embedding MATCH ?1 AND k = ?2
         )
         SELECT c.note_id, knn.distance, c.text
         FROM knn JOIN chunks c ON c.id = knn.chunk_id
         ORDER BY knn.distance",
    )?;

    // Best (min-distance) chunk per note.
    let mut best: HashMap<i64, (f64, String)> = HashMap::new();
    let rows = stmt.query_map(params![vec::f32_blob(&qvec), k], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?, r.get::<_, String>(2)?))
    })?;
    for row in rows {
        let (note_id, distance, text) = row?;
        best.entry(note_id)
            .and_modify(|e| {
                if distance < e.0 {
                    *e = (distance, text.clone());
                }
            })
            .or_insert((distance, text));
    }

    let mut ranked: Vec<(i64, f64, String)> =
        best.into_iter().map(|(id, (d, t))| (id, d, t)).collect();
    ranked.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    ranked.truncate(limit);

    let mut hits = Vec::with_capacity(ranked.len());
    for (note_id, distance, chunk_text) in ranked {
        if let Some((rel_path, title)) = conn
            .query_row(
                "SELECT rel_path, title FROM notes WHERE id = ?1",
                [note_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?
        {
            let snippet: String = chunk_text.chars().take(160).collect();
            hits.push(SearchHit {
                note_id,
                rel_path,
                title,
                snippet,
                score: score_from_distance(distance),
                source: HitSource::Semantic,
            });
        }
    }
    Ok(hits)
}

/// Hybrid search: reciprocal-rank-fusion of full-text and semantic results.
pub fn search_hybrid(
    conn: &Connection,
    embedder: &Embedder,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    const RRF_K: f64 = 60.0;
    let pool = (limit * 3).max(limit);
    let fts_hits = fts::search_fulltext(conn, query, pool)?;
    let sem_hits = search_semantic(conn, embedder, query, pool)?;

    // note_id -> (rrf score, representative hit)
    let mut merged: HashMap<i64, (f64, SearchHit)> = HashMap::new();
    let mut fold = |hits: Vec<SearchHit>| {
        for (rank, hit) in hits.into_iter().enumerate() {
            let contrib = 1.0 / (RRF_K + rank as f64 + 1.0);
            merged
                .entry(hit.note_id)
                .and_modify(|(score, existing)| {
                    *score += contrib;
                    // Prefer a non-empty FTS snippet (has match markers).
                    if existing.snippet.is_empty() {
                        existing.snippet = hit.snippet.clone();
                    }
                })
                .or_insert((contrib, SearchHit { source: HitSource::Hybrid, ..hit }));
        }
    };
    fold(fts_hits);
    fold(sem_hits);

    let mut ranked: Vec<SearchHit> = merged
        .into_values()
        .map(|(score, mut hit)| {
            hit.score = score;
            hit.source = HitSource::Hybrid;
            hit
        })
        .collect();
    ranked.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    ranked.truncate(limit);
    Ok(ranked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::embed::{default_model_dir, Embedder};
    use crate::index;
    use std::fs;

    fn setup() -> (tempfile::TempDir, Connection, Embedder) {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("cooking.md"),
            "# Cooking\n\nPreparing a delicious meal in the kitchen: chop vegetables, simmer the sauce, and serve dinner.",
        )
        .unwrap();
        fs::write(
            dir.path().join("finance.md"),
            "# Finance\n\nThe quarterly earnings report shows revenue growth and profit margins for shareholders.",
        )
        .unwrap();
        let conn = db::open_in_memory().unwrap();
        index::reindex_all(&conn, dir.path()).unwrap();
        let embedder = Embedder::from_dir(&default_model_dir()).unwrap();
        embed_all(&conn, &embedder).unwrap();
        (dir, conn, embedder)
    }

    #[test]
    fn semantic_finds_conceptual_match_without_shared_words() {
        let (_d, conn, embedder) = setup();
        // No literal word overlap with the cooking note.
        let hits = search_semantic(&conn, &embedder, "recipe for supper", 5).unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].rel_path, "cooking.md", "got {:?}", hits);
    }

    #[test]
    fn reindex_replaces_chunks() {
        let (dir, conn, embedder) = setup();
        let note_id = index::note_id_by_path(&conn, "cooking.md").unwrap().unwrap();
        let before: i64 = conn
            .query_row("SELECT count(*) FROM chunks WHERE note_id=?1", [note_id], |r| r.get(0))
            .unwrap();
        assert!(before >= 1);

        fs::write(dir.path().join("cooking.md"), "# Cooking\n\ntiny").unwrap();
        index::reindex_all(&conn, dir.path()).unwrap();
        index_note_embeddings(&conn, &embedder, note_id).unwrap();

        let vec_rows: i64 = conn
            .query_row(
                "SELECT count(*) FROM vec_chunks WHERE chunk_id IN (SELECT id FROM chunks WHERE note_id=?1)",
                [note_id],
                |r| r.get(0),
            )
            .unwrap();
        let chunk_rows: i64 = conn
            .query_row("SELECT count(*) FROM chunks WHERE note_id=?1", [note_id], |r| r.get(0))
            .unwrap();
        assert_eq!(vec_rows, chunk_rows, "vectors stay in sync with chunks");
    }

    #[test]
    fn hybrid_merges_both_engines() {
        let (_d, conn, embedder) = setup();
        let hits = search_hybrid(&conn, &embedder, "kitchen dinner", 5).unwrap();
        assert!(!hits.is_empty());
        assert!(hits.iter().all(|h| h.source == HitSource::Hybrid));
        assert_eq!(hits[0].rel_path, "cooking.md");
    }
}
