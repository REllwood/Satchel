//! Search services over the index: full-text (FTS5), semantic (vectors), and a
//! hybrid merge. Pure query logic — callers (app/CLI/MCP) own the connection.

pub mod fts;
pub mod semantic;

use serde::{Deserialize, Serialize};

/// Start/end of a matched term inside [`SearchHit::snippet`]. Control
/// characters never appear in notes, so the UI can highlight matches without
/// confusing them with Markdown brackets.
pub const MARK_START: char = '\u{2}';
pub const MARK_END: char = '\u{3}';

/// A search result row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchHit {
    pub note_id: i64,
    pub rel_path: String,
    pub title: String,
    /// Snippet with matches wrapped in [`MARK_START`]/[`MARK_END`], or a
    /// leading excerpt.
    pub snippet: String,
    /// Higher is more relevant.
    pub score: f64,
    /// Which engine produced (or contributed to) this hit.
    pub source: HitSource,
}

impl SearchHit {
    /// The same hit with matches shown as `[term]`, for text output (CLI, MCP).
    pub fn with_bracket_marks(mut self) -> Self {
        self.snippet = self.snippet.replace(MARK_START, "[").replace(MARK_END, "]");
        self
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HitSource {
    FullText,
    Semantic,
    Hybrid,
}
