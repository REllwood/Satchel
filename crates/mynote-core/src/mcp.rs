//! Minimal MCP server over stdio. MCP is newline-delimited JSON-RPC 2.0, so we
//! implement it directly (no async runtime / SDK) and expose note tools over a
//! vault. Served by both the `mynote` CLI and the desktop app (`MyNote mcp`),
//! e.g. `claude mcp add --transport stdio mynote -- <app> mcp --vault <folder>`.

use std::io::{self, BufRead, Write};
use std::path::Path;

use anyhow::{bail, Result};
use rusqlite::Connection;
use serde_json::{json, Value};

use crate::search::semantic;
use crate::{db, embed, index, query, search, vault};

/// Serve MCP on stdin/stdout until EOF.
pub fn run(root: &Path) -> Result<()> {
    let db_path = crate::paths::index_db_path(root);
    let conn = db::open(&db_path)?;
    index::reindex_all(&conn, root)?;
    let mut embedder: Option<embed::Embedder> = None;

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let Some(id) = msg.get("id").cloned() else {
            continue; // notification — no response
        };
        let params = msg.get("params").cloned().unwrap_or(Value::Null);

        let response = match method {
            "initialize" => {
                let pv = params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or("2024-11-05")
                    .to_string();
                ok(&id, json!({
                    "protocolVersion": pv,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "mynote", "version": env!("CARGO_PKG_VERSION") }
                }))
            }
            "ping" => ok(&id, json!({})),
            "tools/list" => ok(&id, json!({ "tools": tool_specs() })),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                match call_tool(root, &db_path, &conn, &mut embedder, name, &args) {
                    Ok(text) => ok(&id, json!({
                        "content": [{ "type": "text", "text": text }],
                        "isError": false
                    })),
                    Err(e) => ok(&id, json!({
                        "content": [{ "type": "text", "text": format!("Error: {e}") }],
                        "isError": true
                    })),
                }
            }
            _ => err(&id, -32601, "Method not found"),
        };

        writeln!(out, "{}", serde_json::to_string(&response)?)?;
        out.flush()?;
    }
    Ok(())
}

fn ok(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn err(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing string argument '{key}'"))
}

fn limit_arg(args: &Value) -> usize {
    args.get("limit")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(20)
}

fn tool_specs() -> Value {
    let q = json!({
        "type": "object",
        "properties": {
            "query": { "type": "string" },
            "limit": { "type": "number", "description": "max results (default 20)" }
        },
        "required": ["query"]
    });
    json!([
        { "name": "search_notes", "description": "Full-text search over note titles and bodies.", "inputSchema": q },
        { "name": "semantic_search", "description": "Meaning-based (vector) search over notes.", "inputSchema": q },
        { "name": "list_notes", "description": "List all notes (rel_path + title).", "inputSchema": { "type": "object", "properties": {} } },
        { "name": "read_note", "description": "Read a note's Markdown by vault-relative path.", "inputSchema": { "type": "object", "properties": { "path": { "type": "string" } }, "required": ["path"] } },
        { "name": "create_note", "description": "Create a note with a title and optional Markdown content.", "inputSchema": { "type": "object", "properties": { "title": { "type": "string" }, "content": { "type": "string" } }, "required": ["title"] } },
        { "name": "list_tags", "description": "List tags with note counts.", "inputSchema": { "type": "object", "properties": {} } },
        { "name": "run_query", "description": "Run a read-only SQL SELECT over the index (tables: notes, links, tags, attachments, chunks).", "inputSchema": { "type": "object", "properties": { "sql": { "type": "string" } }, "required": ["sql"] } }
    ])
}

fn call_tool(
    root: &Path,
    db_path: &Path,
    conn: &Connection,
    embedder: &mut Option<embed::Embedder>,
    name: &str,
    args: &Value,
) -> Result<String> {
    // The server is long-lived while notes change underneath it (the app,
    // cloud sync, other agents); an incremental reindex is a cheap stat walk.
    if matches!(name, "search_notes" | "semantic_search" | "list_notes" | "list_tags" | "run_query") {
        index::reindex_all(conn, root)?;
    }
    match name {
        "search_notes" => {
            let hits = search::fts::search_fulltext(conn, str_arg(args, "query")?, limit_arg(args))?;
            Ok(serde_json::to_string_pretty(&hits)?)
        }
        "semantic_search" => {
            if embedder.is_none() {
                *embedder = Some(embed::Embedder::from_dir(&embed::default_model_dir())?);
            }
            let emb = embedder.as_ref().unwrap();
            // Embed notes added or edited since the last search.
            semantic::embed_pending(conn, emb)?;
            let hits = semantic::search_semantic(conn, emb, str_arg(args, "query")?, limit_arg(args))?;
            Ok(serde_json::to_string_pretty(&hits)?)
        }
        "list_notes" => {
            let mut stmt = conn.prepare("SELECT rel_path, title FROM notes ORDER BY rel_path")?;
            let rows: Vec<Value> = stmt
                .query_map([], |r| {
                    Ok(json!({ "rel_path": r.get::<_, String>(0)?, "title": r.get::<_, String>(1)? }))
                })?
                .collect::<std::result::Result<_, _>>()?;
            Ok(serde_json::to_string_pretty(&rows)?)
        }
        "read_note" => {
            let abs = vault::safe_join(root, str_arg(args, "path")?)?;
            Ok(vault::read_note(&abs)?)
        }
        "create_note" => {
            let title = str_arg(args, "title")?;
            let safe = title.replace(['/', '\\'], "-");
            let rel = format!("{safe}.md");
            let abs = vault::safe_join(root, &rel)?;
            if abs.exists() {
                bail!("note already exists: {rel}");
            }
            let content = args
                .get("content")
                .and_then(Value::as_str)
                .map(|c| c.to_string())
                .unwrap_or_else(|| format!("# {title}\n\n"));
            vault::write_note_atomic(&abs, &content)?;
            index::index_single(conn, root, &rel)?;
            Ok(rel)
        }
        "list_tags" => {
            let mut stmt =
                conn.prepare("SELECT tag, count(*) c FROM tags GROUP BY tag ORDER BY c DESC, tag")?;
            let rows: Vec<Value> = stmt
                .query_map([], |r| {
                    Ok(json!({ "tag": r.get::<_, String>(0)?, "count": r.get::<_, i64>(1)? }))
                })?
                .collect::<std::result::Result<_, _>>()?;
            Ok(serde_json::to_string_pretty(&rows)?)
        }
        "run_query" => {
            let qconn = query::open_query_connection(db_path)?;
            let result = query::run_query(&qconn, str_arg(args, "sql")?, 1000)?;
            Ok(serde_json::to_string_pretty(&result)?)
        }
        other => bail!("unknown tool: {other}"),
    }
}
