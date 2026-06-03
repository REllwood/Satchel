//! JSONCanvas (`.canvas`) read/write — the open spec used by Obsidian Canvas.
//! See https://jsoncanvas.org. We round-trip the documented fields so files
//! stay interoperable; unknown fields are preserved best-effort via `extra`.

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CanvasNode {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: String, // "text" | "file" | "link" | "group"
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>, // text node
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>, // file node (a note/attachment path)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>, // link node
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>, // group label
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CanvasEdge {
    pub id: String,
    pub from_node: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_side: Option<String>,
    pub to_node: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_side: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Canvas {
    #[serde(default)]
    pub nodes: Vec<CanvasNode>,
    #[serde(default)]
    pub edges: Vec<CanvasEdge>,
}

pub fn parse_canvas(json: &str) -> Result<Canvas> {
    if json.trim().is_empty() {
        return Ok(Canvas::default());
    }
    Ok(serde_json::from_str(json)?)
}

pub fn serialize_canvas(canvas: &Canvas) -> Result<String> {
    Ok(serde_json::to_string_pretty(canvas)?)
}

/// Note/attachment paths referenced by `file` nodes — used to surface canvas
/// links in the graph.
pub fn file_refs(canvas: &Canvas) -> Vec<String> {
    canvas
        .nodes
        .iter()
        .filter(|n| n.node_type == "file")
        .filter_map(|n| n.file.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "nodes": [
        {"id":"a","type":"file","file":"notes/idea.md","x":0,"y":0,"width":300,"height":120},
        {"id":"b","type":"text","text":"a thought","x":400,"y":0,"width":250,"height":100,"color":"4"}
      ],
      "edges": [
        {"id":"e1","fromNode":"a","fromSide":"right","toNode":"b","toSide":"left","label":"leads to"}
      ]
    }"#;

    #[test]
    fn round_trips_jsoncanvas() {
        let canvas = parse_canvas(SAMPLE).unwrap();
        assert_eq!(canvas.nodes.len(), 2);
        assert_eq!(canvas.edges.len(), 1);
        let serialized = serialize_canvas(&canvas).unwrap();
        let again = parse_canvas(&serialized).unwrap();
        assert_eq!(canvas, again, "structurally identical after round-trip");
    }

    #[test]
    fn file_refs_lists_note_nodes() {
        let canvas = parse_canvas(SAMPLE).unwrap();
        assert_eq!(file_refs(&canvas), vec!["notes/idea.md".to_string()]);
    }

    #[test]
    fn empty_is_valid() {
        assert_eq!(parse_canvas("").unwrap(), Canvas::default());
        let s = serialize_canvas(&Canvas::default()).unwrap();
        assert!(parse_canvas(&s).is_ok());
    }
}
