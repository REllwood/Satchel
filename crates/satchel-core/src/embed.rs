//! On-device text embeddings via fastembed-rs (ONNX Runtime), plus note
//! chunking. Fully offline: the model is loaded from local files, nothing is
//! fetched at runtime.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use fastembed::{
    InitOptionsUserDefined, Pooling, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel,
};

/// Dimensionality of all-MiniLM-L6-v2 / bge-small embeddings.
pub const EMBEDDING_DIM: usize = 384;

/// Default bundled model id.
pub const DEFAULT_MODEL: &str = "all-MiniLM-L6-v2";

/// A chunk of a note's body to be embedded independently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub ord: usize,
    /// Byte offsets into the note body.
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// Loaded embedding model. Cheap to clone is *not* implied — hold one per vault.
pub struct Embedder {
    model: Mutex<TextEmbedding>,
    dim: usize,
}

impl Embedder {
    /// Load a model from a directory containing `model.onnx`, `tokenizer.json`,
    /// `config.json`, `tokenizer_config.json`, `special_tokens_map.json`.
    pub fn from_dir(dir: &Path) -> Result<Self> {
        let read = |name: &str| -> Result<Vec<u8>> {
            std::fs::read(dir.join(name)).with_context(|| format!("reading model file {name}"))
        };
        let onnx = read("model.onnx")?;
        let tokenizer_files = TokenizerFiles {
            tokenizer_file: read("tokenizer.json")?,
            config_file: read("config.json")?,
            special_tokens_map_file: read("special_tokens_map.json")?,
            tokenizer_config_file: read("tokenizer_config.json")?,
        };
        // MiniLM/bge use mean pooling (fastembed defaults to CLS).
        let model =
            UserDefinedEmbeddingModel::new(onnx, tokenizer_files).with_pooling(Pooling::Mean);
        let embedding = TextEmbedding::try_new_from_user_defined(
            model,
            InitOptionsUserDefined::default(),
        )
        .context("initializing ONNX embedding model")?;
        Ok(Self {
            model: Mutex::new(embedding),
            dim: EMBEDDING_DIM,
        })
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Embed a batch of texts. Returns one L2-normalized vector per input.
    pub fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let mut model = self.model.lock().expect("embedder mutex poisoned");
        let vectors = model.embed(refs, None)?;
        Ok(vectors)
    }

    pub fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        Ok(self
            .embed(std::slice::from_ref(&text.to_string()))?
            .into_iter()
            .next()
            .unwrap_or_default())
    }
}

/// Resolve the bundled model directory for the running binary.
///
/// Order: `SATCHEL_MODEL_DIR`; the model shipped alongside this executable
/// (macOS `.app` Resources, next to the binary, Linux package lib dir); an
/// installed `Satchel.app` (so the CLI can reuse the app's model); finally the
/// source checkout (dev and tests). The desktop app passes Tauri's resolved
/// resource path explicitly.
pub fn default_model_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("SATCHEL_MODEL_DIR") {
        return PathBuf::from(dir);
    }
    let rel = Path::new("models").join(DEFAULT_MODEL);
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        candidates.push(exe_dir.join("../Resources").join(&rel));
        candidates.push(exe_dir.join(&rel));
        candidates.push(exe_dir.join("../lib/Satchel").join(&rel));
    }
    #[cfg(target_os = "macos")]
    candidates.push(Path::new("/Applications/Satchel.app/Contents/Resources").join(&rel));

    let dev = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets").join(&rel);
    candidates
        .into_iter()
        .find(|dir| dir.join("model.onnx").is_file())
        .unwrap_or(dev)
}

/// Split a note body into chunks (~1200 chars), breaking at heading boundaries
/// so each chunk is a coherent unit. Byte offsets index into `body`.
pub fn chunk_text(body: &str) -> Vec<Chunk> {
    const TARGET_CHARS: usize = 1200;
    let mut chunks = Vec::new();
    let mut ord = 0usize;
    let mut chunk_start = 0usize;
    let mut chunk_chars = 0usize;
    let mut buf = String::new();
    let mut offset = 0usize;

    let mut flush = |start: usize, end: usize, text: &str, ord: &mut usize| {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            chunks.push(Chunk {
                ord: *ord,
                start,
                end,
                text: trimmed.to_string(),
            });
            *ord += 1;
        }
    };

    for line in body.split_inclusive('\n') {
        let is_heading = line.trim_start().starts_with('#');
        // A heading starts a new chunk if we've already accumulated content.
        if is_heading && !buf.trim().is_empty() {
            flush(chunk_start, offset, &buf, &mut ord);
            buf.clear();
            chunk_chars = 0;
            chunk_start = offset;
        }
        buf.push_str(line);
        chunk_chars += line.chars().count();
        offset += line.len();
        if chunk_chars >= TARGET_CHARS {
            flush(chunk_start, offset, &buf, &mut ord);
            buf.clear();
            chunk_chars = 0;
            chunk_start = offset;
        }
    }
    flush(chunk_start, offset, &buf, &mut ord);
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        dot / (na * nb)
    }

    #[test]
    fn chunks_short_note_into_one() {
        let chunks = chunk_text("# Title\n\njust a little text");
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].text.contains("little text"));
    }

    #[test]
    fn chunks_long_note_into_several() {
        let body = "para\n\n".repeat(400); // well over the char target
        let chunks = chunk_text(&body);
        assert!(chunks.len() > 1, "expected multiple chunks, got {}", chunks.len());
        // Offsets are ordered and within bounds.
        for w in chunks.windows(2) {
            assert!(w[0].start <= w[1].start);
        }
        assert!(chunks.last().unwrap().end <= body.len());
    }

    #[test]
    fn embeds_offline_and_captures_similarity() {
        // Uses the bundled model; runs fully offline.
        let embedder = Embedder::from_dir(&default_model_dir()).expect("load model");
        let vecs = embedder
            .embed(&[
                "the cat sat on the mat".to_string(),
                "a kitten rested on the rug".to_string(),
                "quarterly financial earnings report".to_string(),
            ])
            .expect("embed");
        assert_eq!(vecs.len(), 3);
        assert_eq!(vecs[0].len(), EMBEDDING_DIM);

        let sim_related = cosine(&vecs[0], &vecs[1]);
        let sim_unrelated = cosine(&vecs[0], &vecs[2]);
        assert!(
            sim_related > sim_unrelated,
            "related ({sim_related}) should beat unrelated ({sim_unrelated})"
        );
    }
}
