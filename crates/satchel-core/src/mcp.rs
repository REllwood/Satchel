//! Minimal MCP server over stdio. MCP is newline-delimited JSON-RPC 2.0, so we
//! implement it directly (no async runtime / SDK) and expose note tools over a
//! vault. Served by both the `satchel` CLI and the desktop app (`Satchel mcp`),
//! e.g. `claude mcp add --transport stdio satchel -- <app> mcp --vault <folder>`.

use std::io::{self, BufRead, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use rusqlite::Connection;
use serde_json::{json, Value};

use crate::search::semantic;
use crate::{db, embed, index, query, search, vault};

/// MCP protocol versions this server speaks, newest first.
const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Largest note `read_note` returns in full.
const MAX_NOTE_BYTES: usize = 1024 * 1024;

/// How long one `semantic_search` call spends embedding new notes before it
/// searches what is ready, so the first call on a big vault cannot hang.
const EMBED_BUDGET: Duration = Duration::from_secs(8);

/// Serve MCP on stdin/stdout until EOF.
pub fn run(root: &Path) -> Result<()> {
    if !root.is_dir() {
        bail!("notes folder not found: {}", root.display());
    }
    let db_path = crate::paths::index_db_path(root);
    let conn = db::open(&db_path)?;
    // No reindex here: every tool that reads the index refreshes it first, so
    // `initialize` answers at once even on a large vault.
    let mut embedder: Option<embed::Embedder> = None;

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut input = stdin.lock();
    let mut buf = Vec::new();

    loop {
        buf.clear();
        if input.read_until(b'\n', &mut buf)? == 0 {
            break;
        }
        let line = String::from_utf8_lossy(&buf);
        if line.trim().is_empty() {
            continue;
        }
        let msg = match serde_json::from_str::<Value>(&line) {
            Ok(msg) if msg.is_object() => msg,
            Ok(Value::Array(_)) => {
                send(&mut out, &err(&Value::Null, -32600, "Batch requests are not supported"))?;
                continue;
            }
            _ => {
                send(&mut out, &err(&Value::Null, -32700, "Parse error"))?;
                continue;
            }
        };
        let Some(method) = msg.get("method").and_then(Value::as_str) else {
            continue; // a response to something we sent — we send no requests
        };
        let Some(id) = msg.get("id").cloned() else {
            continue; // notification — no response
        };
        let params = msg.get("params").cloned().unwrap_or(Value::Null);

        let response = match method {
            "initialize" => {
                let requested = params.get("protocolVersion").and_then(Value::as_str);
                let pv = requested
                    .filter(|v| PROTOCOL_VERSIONS.contains(v))
                    .unwrap_or(PROTOCOL_VERSIONS[0]);
                ok(&id, json!({
                    "protocolVersion": pv,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "satchel", "version": env!("CARGO_PKG_VERSION") }
                }))
            }
            "ping" => ok(&id, json!({})),
            "tools/list" => ok(&id, json!({ "tools": tool_specs() })),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                match call_tool(root, &db_path, &conn, &mut embedder, name, &args) {
                    Ok(texts) => ok(&id, json!({
                        "content": texts
                            .into_iter()
                            .map(|text| json!({ "type": "text", "text": text }))
                            .collect::<Vec<_>>(),
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

        send(&mut out, &response)?;
    }
    Ok(())
}

fn send(out: &mut impl Write, message: &Value) -> Result<()> {
    writeln!(out, "{}", serde_json::to_string(message)?)?;
    out.flush()?;
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

/// A count argument, clamped to `1..=max`.
fn count_arg(args: &Value, key: &str, default: usize, max: usize) -> usize {
    args.get(key)
        .and_then(Value::as_u64)
        .map(|n| (n.min(max as u64) as usize).max(1))
        .unwrap_or(default)
}

fn limit_arg(args: &Value) -> usize {
    count_arg(args, "limit", 20, 100)
}

fn tool_specs() -> Value {
    let q = json!({
        "type": "object",
        "properties": {
            "query": { "type": "string" },
            "limit": { "type": "number", "description": "max results (default 20, at most 100)" }
        },
        "required": ["query"]
    });
    json!([
        { "name": "search_notes", "description": "Full-text search over note titles and bodies.", "inputSchema": q },
        { "name": "semantic_search", "description": "Meaning-based (vector) search over notes.", "inputSchema": q },
        { "name": "list_notes", "description": "List notes (rel_path + title), a page at a time. Returns total and next_offset (null on the last page).", "inputSchema": { "type": "object", "properties": {
            "folder": { "type": "string", "description": "only notes in this folder (vault-relative)" },
            "offset": { "type": "number", "description": "skip this many notes (default 0)" },
            "limit": { "type": "number", "description": "page size (default 200, at most 1000)" }
        } } },
        { "name": "read_note", "description": "Read a note's Markdown by vault-relative path (a .md note or .canvas file).", "inputSchema": { "type": "object", "properties": { "path": { "type": "string" } }, "required": ["path"] } },
        { "name": "create_note", "description": "Create a note with a title and optional Markdown content, optionally in a folder. Fails if a note with that name exists.", "inputSchema": { "type": "object", "properties": { "title": { "type": "string" }, "content": { "type": "string" }, "folder": { "type": "string", "description": "vault-relative folder, created if missing" } }, "required": ["title"] } },
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
) -> Result<Vec<String>> {
    // The server is long-lived while notes change underneath it (the app,
    // cloud sync, other agents); an incremental reindex is a cheap stat walk.
    if matches!(name, "search_notes" | "semantic_search" | "list_notes" | "list_tags" | "run_query") {
        index::reindex_all(conn, root)?;
    }
    let one = |text: String| Ok(vec![text]);
    match name {
        "search_notes" => {
            let hits: Vec<_> = search::fts::search_fulltext(conn, str_arg(args, "query")?, limit_arg(args))?
                .into_iter()
                .map(search::SearchHit::with_bracket_marks)
                .collect();
            one(serde_json::to_string_pretty(&hits)?)
        }
        "semantic_search" => {
            if embedder.is_none() {
                *embedder = Some(embed::Embedder::from_dir(&embed::default_model_dir())?);
            }
            let emb = embedder.as_ref().unwrap();
            semantic::ensure_semantic(conn, emb.dim() as u32)?;
            // Embed notes added or edited since the last search, within a
            // time budget so the first search on a big vault cannot hang.
            let started = Instant::now();
            let mut waiting = 0;
            for id in semantic::pending_notes(conn)? {
                if started.elapsed() > EMBED_BUDGET {
                    waiting += 1;
                    continue;
                }
                semantic::index_note_embeddings(conn, emb, id)?;
            }
            let hits: Vec<_> = semantic::search_semantic(conn, emb, str_arg(args, "query")?, limit_arg(args))?
                .into_iter()
                .map(search::SearchHit::with_bracket_marks)
                .collect();
            let mut out = vec![serde_json::to_string_pretty(&hits)?];
            if waiting > 0 {
                out.push(format!(
                    "{waiting} notes are not indexed for semantic search yet, so they are missing from these results. Search again to index more."
                ));
            }
            Ok(out)
        }
        "list_notes" => {
            let folder = args.get("folder").and_then(Value::as_str).unwrap_or("").trim_matches('/');
            let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
            let limit = count_arg(args, "limit", 200, 1000);
            let mut stmt = conn.prepare(
                "SELECT rel_path, title FROM notes
                 WHERE ?1 = '' OR substr(rel_path, 1, length(?1) + 1) = ?1 || '/'
                 ORDER BY rel_path",
            )?;
            let all: Vec<Value> = stmt
                .query_map([folder], |r| {
                    Ok(json!({ "rel_path": r.get::<_, String>(0)?, "title": r.get::<_, String>(1)? }))
                })?
                .collect::<std::result::Result<_, _>>()?;
            let total = all.len();
            let page: Vec<Value> = all.into_iter().skip(offset).take(limit).collect();
            let next = (offset + page.len() < total).then_some(offset + page.len());
            one(serde_json::to_string_pretty(&json!({ "total": total, "notes": page, "next_offset": next }))?)
        }
        "read_note" => {
            let rel = str_arg(args, "path")?;
            let lower = rel.to_lowercase();
            if vault::is_hidden_path(rel) || !(lower.ends_with(".md") || lower.ends_with(".canvas")) {
                bail!("only notes (.md) and canvases (.canvas) can be read");
            }
            let abs = vault::safe_join(root, rel)?;
            vault::ensure_local(&abs, Duration::from_secs(20))?;
            let text = vault::read_note(&abs)?;
            if text.len() <= MAX_NOTE_BYTES {
                return one(text);
            }
            let mut end = MAX_NOTE_BYTES;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            Ok(vec![
                text[..end].to_string(),
                format!("This note is {} bytes long. Only the first {end} are shown.", text.len()),
            ])
        }
        "create_note" => {
            let title = str_arg(args, "title")?;
            let folder = args.get("folder").and_then(Value::as_str).unwrap_or("").trim_matches('/');
            let name = format!("{}.md", vault::note_file_name(title));
            let rel = if folder.is_empty() { name } else { format!("{folder}/{name}") };
            if vault::is_hidden_path(&rel) {
                bail!("notes cannot go in hidden folders");
            }
            let abs = vault::safe_join(root, &rel)?;
            let content = args
                .get("content")
                .and_then(Value::as_str)
                .map(|c| c.to_string())
                .unwrap_or_else(|| format!("# {title}\n\n"));
            match vault::create_file_exclusive(&abs, content.as_bytes()) {
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => bail!("note already exists: {rel}"),
                result => result?,
            }
            index::index_single(conn, root, &rel)?;
            one(rel)
        }
        "list_tags" => {
            let mut stmt =
                conn.prepare("SELECT tag, count(*) c FROM tags GROUP BY tag ORDER BY c DESC, tag")?;
            let rows: Vec<Value> = stmt
                .query_map([], |r| {
                    Ok(json!({ "tag": r.get::<_, String>(0)?, "count": r.get::<_, i64>(1)? }))
                })?
                .collect::<std::result::Result<_, _>>()?;
            one(serde_json::to_string_pretty(&rows)?)
        }
        "run_query" => {
            let qconn = query::open_query_connection(db_path)?;
            let result = query::run_query(&qconn, str_arg(args, "sql")?, 1000)?;
            one(serde_json::to_string_pretty(&result)?)
        }
        other => bail!("unknown tool: {other}"),
    }
}
