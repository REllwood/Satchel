//! Search services over the index: full-text (FTS5), semantic (vectors), and a
//! hybrid merge. Pure query logic — callers (app/CLI/MCP) own the connection.

pub mod fts;
pub mod semantic;

use serde::{Deserialize, Serialize};

/// A search result row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchHit {
    pub note_id: i64,
    pub rel_path: String,
    pub title: String,
    /// Snippet with match markers (`[...]`), or a leading excerpt.
    pub snippet: String,
    /// Higher is more relevant.
    pub score: f64,
    /// Which engine produced (or contributed to) this hit.
    pub source: HitSource,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HitSource {
    FullText,
    Semantic,
    Hybrid,
}
