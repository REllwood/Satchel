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

/// A note's text split into chunks, ready to embed. Embedding is the slow
/// part, so a caller sharing the connection can prepare a job, release the
/// connection while it runs [`EmbeddingJob::texts`] through the model, then
/// store the vectors with [`store_note_embeddings`].
pub struct EmbeddingJob {
    pub note_id: i64,
    /// Content hash when prepared, so vectors of outdated text are never stored.
    hash: String,
    chunks: Vec<embed::Chunk>,
}

impl EmbeddingJob {
    /// The texts to embed, one per chunk.
    pub fn texts(&self) -> Vec<String> {
        self.chunks.iter().map(|c| c.text.clone()).collect()
    }
}

/// Read a note and chunk it for embedding. `None` if the note no longer exists.
/// A note whose contents are only in the cloud gets a job with no chunks.
pub fn prepare_note_embeddings(conn: &Connection, note_id: i64) -> Result<Option<EmbeddingJob>> {
    let row: Option<(String, String, bool)> = conn
        .query_row(
            "SELECT body, hash, materialized FROM notes WHERE id = ?1",
            [note_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0)),
        )
        .optional()?;
    let Some((body, hash, materialized)) = row else {
        return Ok(None);
    };
    let chunks = if materialized {
        embed::chunk_text(&body)
    } else {
        Vec::new()
    };
    Ok(Some(EmbeddingJob { note_id, hash, chunks }))
}

/// Replace a note's chunks and vectors with a job's results. Returns false,
/// storing nothing, if the note changed or was removed after the job was
/// prepared. A changed note is left pending, so a later pass embeds its new text.
pub fn store_note_embeddings(
    conn: &Connection,
    job: &EmbeddingJob,
    vectors: &[Vec<f32>],
) -> Result<bool> {
    // The hash check and the writes form one transaction, so a note saved by
    // another process in between cannot get the old text's vectors.
    crate::db::atomically(conn, || replace_note_chunks(conn, job, vectors))
}

fn replace_note_chunks(conn: &Connection, job: &EmbeddingJob, vectors: &[Vec<f32>]) -> Result<bool> {
    let hash: Option<String> = conn
        .query_row("SELECT hash FROM notes WHERE id = ?1", [job.note_id], |r| r.get(0))
        .optional()?;
    if hash.as_deref() != Some(job.hash.as_str()) {
        return Ok(false);
    }

    // Clear any previous chunks/vectors for this note.
    vec::delete_note_vectors(conn, job.note_id)?;
    conn.execute("DELETE FROM chunks WHERE note_id = ?1", [job.note_id])?;

    for (chunk, vector) in job.chunks.iter().zip(vectors.iter()) {
        let chunk_id: i64 = conn.query_row(
            "INSERT INTO chunks (note_id, ord, start, \"end\", text) VALUES (?1, ?2, ?3, ?4, ?5) RETURNING id",
            params![job.note_id, chunk.ord as i64, chunk.start as i64, chunk.end as i64, chunk.text],
            |r| r.get(0),
        )?;
        conn.execute(
            "INSERT INTO vec_chunks (chunk_id, embedding) VALUES (?1, ?2)",
            params![chunk_id, vec::f32_blob(vector)],
        )?;
    }
    Ok(true)
}

/// (Re)embed a single note's chunks into `chunks` + `vec_chunks`.
/// Returns the number of chunks embedded.
pub fn index_note_embeddings(conn: &Connection, embedder: &Embedder, note_id: i64) -> Result<usize> {
    let Some(job) = prepare_note_embeddings(conn, note_id)? else {
        return Ok(0);
    };
    let vectors = embedder.embed(&job.texts())?;
    store_note_embeddings(conn, &job, &vectors)?;
    Ok(job.chunks.len())
}

/// Notes that have content but no embeddings yet (new, or edited since they
/// were last embedded — the indexer drops chunks when content changes).
pub fn pending_notes(conn: &Connection) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT n.id FROM notes n
         WHERE n.materialized = 1
           AND trim(n.body, ' ' || char(9) || char(10) || char(13)) != ''
           AND NOT EXISTS (SELECT 1 FROM chunks c WHERE c.note_id = n.id)
         ORDER BY n.mtime DESC",
    )?;
    let ids = stmt
        .query_map([], |r| r.get(0))?
        .collect::<std::result::Result<Vec<i64>, _>>()?;
    Ok(ids)
}

/// Embed only what's missing (see [`pending_notes`]). Returns chunks embedded.
pub fn embed_pending(conn: &Connection, embedder: &Embedder) -> Result<usize> {
    ensure_semantic(conn, embedder.dim() as u32)?;
    let mut total = 0;
    for id in pending_notes(conn)? {
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
    search_semantic_vec(conn, &qvec, limit)
}

/// Most results any search returns. sqlite-vec caps a KNN query at 4096 rows,
/// and each note can match through several chunks.
pub const MAX_LIMIT: usize = 1000;

/// [`search_semantic`] with the query already embedded, so a caller can run
/// the model without holding its database lock.
pub fn search_semantic_vec(conn: &Connection, qvec: &[f32], limit: usize) -> Result<Vec<SearchHit>> {
    let limit = limit.clamp(1, MAX_LIMIT);
    let k = (limit.saturating_mul(4)).clamp(10, 4096) as i64;

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
    let rows = stmt.query_map(params![vec::f32_blob(qvec), k], |r| {
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
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let qvec = embedder.embed_one(query)?;
    search_hybrid_vec(conn, query, &qvec, limit)
}

/// [`search_hybrid`] with the query already embedded.
pub fn search_hybrid_vec(conn: &Connection, query: &str, qvec: &[f32], limit: usize) -> Result<Vec<SearchHit>> {
    const RRF_K: f64 = 60.0;
    let limit = limit.clamp(1, MAX_LIMIT);
    let pool = limit.saturating_mul(3).min(MAX_LIMIT);
    let fts_hits = fts::search_fulltext(conn, query, pool)?;
    let sem_hits = search_semantic_vec(conn, qvec, pool)?;

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
        embed_pending(&conn, &embedder).unwrap();
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
    fn pending_notes_tracks_what_needs_embedding() {
        let (dir, conn, embedder) = setup(); // setup() embeds everything
        assert!(pending_notes(&conn).unwrap().is_empty());

        // Editing a note drops its chunks → it becomes pending again.
        fs::write(dir.path().join("finance.md"), "# Finance\n\nnew quarterly numbers").unwrap();
        index::reindex_all(&conn, dir.path()).unwrap();
        let finance = index::note_id_by_path(&conn, "finance.md").unwrap().unwrap();
        assert_eq!(pending_notes(&conn).unwrap(), vec![finance]);

        index_note_embeddings(&conn, &embedder, finance).unwrap();
        assert!(pending_notes(&conn).unwrap().is_empty());
    }

    #[test]
    fn job_is_stored_only_if_the_note_is_unchanged() {
        let (dir, conn, embedder) = setup();
        let finance = index::note_id_by_path(&conn, "finance.md").unwrap().unwrap();
        let edit = |text: &str| {
            fs::write(dir.path().join("finance.md"), text).unwrap();
            index::index_single(&conn, dir.path(), "finance.md").unwrap();
        };

        // The note is edited again while its previous text is being embedded.
        edit("# Finance\n\nfirst revision");
        let stale = prepare_note_embeddings(&conn, finance).unwrap().unwrap();
        let stale_vectors = embedder.embed(&stale.texts()).unwrap();
        edit("# Finance\n\nsecond, longer revision");
        assert!(!store_note_embeddings(&conn, &stale, &stale_vectors).unwrap());
        assert_eq!(pending_notes(&conn).unwrap(), vec![finance]);

        let job = prepare_note_embeddings(&conn, finance).unwrap().unwrap();
        let vectors = embedder.embed(&job.texts()).unwrap();
        assert!(store_note_embeddings(&conn, &job, &vectors).unwrap());
        assert!(pending_notes(&conn).unwrap().is_empty());
        let stored: String = conn
            .query_row("SELECT text FROM chunks WHERE note_id = ?1", [finance], |r| r.get(0))
            .unwrap();
        assert!(stored.contains("second, longer revision"), "got {stored:?}");
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
