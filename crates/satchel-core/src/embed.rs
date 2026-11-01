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

/// Split a note body into chunks of about 300 model tokens (roughly 1200
/// characters of English), breaking at heading boundaries so each chunk is a
/// coherent unit. A line too long for one chunk is split after a sentence,
/// else at a space, so no text falls past the model's 512-token input limit.
/// Byte offsets index into `body`.
pub fn chunk_text(body: &str) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let mut ord = 0usize;
    let mut chunk_start = 0usize;
    let mut chunk_tokens = 0usize;
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
            chunk_tokens = 0;
            chunk_start = offset;
        }
        for piece in split_long_line(line) {
            let tokens = estimate_tokens(piece);
            if chunk_tokens > 0 && chunk_tokens + tokens > CHUNK_TOKENS {
                flush(chunk_start, offset, &buf, &mut ord);
                buf.clear();
                chunk_tokens = 0;
                chunk_start = offset;
            }
            buf.push_str(piece);
            chunk_tokens += tokens;
            offset += piece.len();
        }
    }
    flush(chunk_start, offset, &buf, &mut ord);
    chunks
}

/// Target chunk size in model tokens. The model reads at most 512.
const CHUNK_TOKENS: usize = 300;

/// Rough token count for MiniLM's tokenizer: each Chinese, Japanese or Korean
/// character is about one token, and other text averages about four
/// characters per token.
fn estimate_tokens(text: &str) -> usize {
    let (wide, other) = text.chars().fold((0usize, 0usize), |(w, o), c| {
        if is_wide_char(c) {
            (w + 1, o)
        } else {
            (w, o + 1)
        }
    });
    wide + other.div_ceil(4)
}

fn is_wide_char(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF
        | 0xF900..=0xFAFF | 0xFF66..=0xFF9F | 0x20000..=0x2FFFF)
}

/// Split one line into consecutive pieces of at most [`CHUNK_TOKENS`], cutting
/// after the end of a sentence where possible, else at a space, else anywhere
/// (always on a character boundary). The pieces join back into `line`.
fn split_long_line(line: &str) -> Vec<&str> {
    if estimate_tokens(line) <= CHUNK_TOKENS {
        return vec![line];
    }
    let mut pieces = Vec::new();
    let mut start = 0usize;
    let mut tokens = 0usize;
    let mut run = 0usize; // non-wide characters since the last token was counted
    let mut sentence_end = None;
    let mut space = None;
    let mut chars = line.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let next = i + c.len_utf8();
        if is_wide_char(c) {
            tokens += 1;
        } else {
            run += 1;
            if run == 4 {
                tokens += 1;
                run = 0;
            }
        }
        let next_char = chars.peek().map(|&(_, n)| n);
        if matches!(c, '。' | '！' | '？')
            || (matches!(c, '.' | '!' | '?') && next_char.is_none_or(char::is_whitespace))
        {
            sentence_end = Some(next);
        } else if c.is_whitespace() {
            space = Some(next);
        }
        if tokens >= CHUNK_TOKENS && next < line.len() {
            let cut = sentence_end.or(space).filter(|&cut| cut > start).unwrap_or(next);
            pieces.push(&line[start..cut]);
            start = cut;
            tokens = estimate_tokens(&line[cut..next]);
            run = 0;
            sentence_end = None;
            space = None;
        }
    }
    if start < line.len() {
        pieces.push(&line[start..]);
    }
    pieces
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
    fn long_single_line_paragraph_is_split_to_fit_the_model() {
        let sentence = "The quarterly review covered hiring, budgets and the roadmap in detail. ";
        let body = format!("{}The submarine periscope needs maintenance.", sentence.repeat(60));
        let chunks = chunk_text(&body);
        assert!(chunks.len() > 1, "a 4,000-character line must not be one chunk");
        for chunk in &chunks {
            assert!(estimate_tokens(&chunk.text) <= CHUNK_TOKENS + 4, "{}", chunk.text.len());
            assert_eq!(&body[chunk.start..chunk.end].trim(), &chunk.text);
        }
        assert!(chunks.last().unwrap().text.contains("periscope"));
        // Cuts land after full sentences, not mid-word.
        assert!(chunks[0].text.ends_with("detail."), "{}", chunks[0].text);
    }

    #[test]
    fn japanese_text_is_split_by_its_token_count() {
        let body = "昨日は東京タワーに行った。".repeat(80); // about 1,000 wide characters
        let chunks = chunk_text(&body);
        assert!(chunks.len() >= 3, "got {}", chunks.len());
        assert!(chunks.iter().all(|c| estimate_tokens(&c.text) <= CHUNK_TOKENS + 4));
        assert!(chunks[0].text.ends_with('。'));
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
