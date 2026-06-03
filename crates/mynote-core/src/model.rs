//! Core domain types. Pure data — no I/O.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// SQLite rowid for a note.
pub type NoteId = i64;

/// A heading extracted from a note body.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    pub text: String,
}

/// Kind of in-note reference.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    /// `[[Target]]`
    Wikilink,
    /// `![[Target]]` (transclusion/embed)
    Embed,
}

/// A `[[wikilink]]` parsed from a note, split into its parts.
/// Obsidian form: `[[target#heading|alias]]`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Link {
    /// Link target, e.g. `Some Note` (without `#heading` or `|alias`).
    pub target: String,
    /// Optional heading anchor (`#heading`).
    pub heading: Option<String>,
    /// Optional display alias (`|alias`).
    pub alias: Option<String>,
    pub kind: LinkKind,
}

/// Output of parsing a note's text. Pure parsing result — no file metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParsedNote {
    /// Best-effort title (frontmatter `title`, else first H1). May be empty —
    /// the caller substitutes the filename stem when so.
    pub title: String,
    /// Frontmatter as JSON (object, or `Value::Null` when absent).
    pub frontmatter: Value,
    /// Markdown body with the frontmatter block removed.
    pub body: String,
    pub headings: Vec<Heading>,
    pub links: Vec<Link>,
    /// De-duplicated tags (from frontmatter `tags` + inline `#tags`), no `#`.
    pub tags: Vec<String>,
    /// Markup-stripped text for full-text indexing and snippets.
    pub plaintext: String,
}

/// A note as tracked in the vault and index.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Note {
    pub id: NoteId,
    /// Path relative to the vault root, always forward-slashed.
    pub rel_path: String,
    pub title: String,
    pub frontmatter: Value,
    pub tags: Vec<String>,
    /// Modification time, unix milliseconds.
    pub mtime: i64,
    pub size: u64,
    /// blake3 hex digest of the file's bytes — the source of truth for change
    /// detection (mtime is unreliable under cloud sync).
    pub hash: String,
}

/// A non-note file referenced by notes (image or other attachment).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Attachment {
    pub rel_path: String,
    pub mime: Option<String>,
}

/// Per-vault configuration, persisted in `.mynote/settings.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct VaultConfig {
    /// Folder (relative to vault) new attachments are written to.
    pub attachments_dir: String,
    /// Embedding model id (drives semantic search; a change forces re-index).
    pub embedding_model: String,
    /// Embedding dimensionality.
    pub embedding_dim: u32,
}

impl Default for VaultConfig {
    fn default() -> Self {
        Self {
            attachments_dir: "attachments".to_string(),
            embedding_model: "all-MiniLM-L6-v2".to_string(),
            embedding_dim: 384,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsed_note_roundtrips_through_json() {
        let parsed = ParsedNote {
            title: "Hello".into(),
            frontmatter: serde_json::json!({ "tags": ["a", "b"] }),
            body: "# Hello\n\nbody".into(),
            headings: vec![Heading { level: 1, text: "Hello".into() }],
            links: vec![Link {
                target: "Other".into(),
                heading: Some("Sec".into()),
                alias: Some("see".into()),
                kind: LinkKind::Wikilink,
            }],
            tags: vec!["a".into(), "b".into()],
            plaintext: "Hello body".into(),
        };
        let json = serde_json::to_string(&parsed).unwrap();
        let back: ParsedNote = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, back);
    }

    #[test]
    fn vault_config_defaults_are_sane() {
        let c = VaultConfig::default();
        assert_eq!(c.embedding_dim, 384);
        assert_eq!(c.attachments_dir, "attachments");
    }
}
