//! `mynote` — headless CLI + MCP server over `mynote-core`.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use mynote_core::rusqlite::Connection;
use mynote_core::search::semantic;
use mynote_core::{agents, db, embed, index, parse, query, search, vault};

#[derive(Parser)]
#[command(
    name = "mynote",
    version,
    about = "MyNote — local-first notes CLI + MCP server"
)]
struct Cli {
    /// Vault folder (default: $MYNOTE_VAULT, else the current directory).
    #[arg(long, global = true)]
    vault: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize a vault: build the index and write AGENTS.md/CLAUDE.md.
    Init,
    /// Rebuild the index from the vault files.
    Index,
    /// Search notes (full-text by default; --semantic for meaning-based).
    Search {
        query: String,
        #[arg(long)]
        semantic: bool,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Run a read-only SQL query over the index.
    Query {
        sql: String,
        #[arg(long)]
        json: bool,
    },
    /// Create a new note.
    New { title: String },
    /// Print a note's content.
    Get { path: String },
    /// Export a note to stdout (Markdown, or rendered --html).
    Export {
        path: String,
        #[arg(long)]
        html: bool,
    },
    /// Run the MCP server over stdio (for Claude Code / Codex).
    Mcp,
}

fn resolve_vault(opt: Option<PathBuf>) -> PathBuf {
    opt.or_else(|| std::env::var_os("MYNOTE_VAULT").map(PathBuf::from))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

/// The per-machine index (shared with the desktop app), outside the vault.
fn db_path(root: &Path) -> PathBuf {
    mynote_core::paths::index_db_path(root)
}

/// Open the index and bring it current (incremental reindex).
fn open_synced(root: &Path) -> Result<Connection> {
    let conn = db::open(&db_path(root)).context("opening index")?;
    index::reindex_all(&conn, root).context("indexing vault")?;
    Ok(conn)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let root = resolve_vault(cli.vault);
    vault::allow_cloud_downloads();

    match cli.command {
        Command::Init => {
            let conn = open_synced(&root)?;
            agents::ensure_agent_docs(&root)?;
            let count: i64 =
                conn.query_row("SELECT count(*) FROM notes", [], |r| r.get(0))?;
            println!("Initialized vault at {} ({count} notes)", root.display());
        }
        Command::Index => {
            let conn = db::open(&db_path(&root))?;
            let stats = index::reindex_all(&conn, &root)?;
            println!(
                "Indexed {} notes (+{} ~{} -{})",
                stats.total, stats.added, stats.updated, stats.removed
            );
        }
        Command::Search {
            query: q,
            semantic: sem,
            limit,
            json,
        } => {
            let conn = open_synced(&root)?;
            let hits = if sem {
                let embedder = embed::Embedder::from_dir(&embed::default_model_dir())
                    .context("loading embedding model")?;
                semantic::embed_pending(&conn, &embedder)?;
                semantic::search_semantic(&conn, &embedder, &q, limit)?
            } else {
                search::fts::search_fulltext(&conn, &q, limit)?
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&hits)?);
            } else {
                for h in hits {
                    println!("{}\t{}", h.rel_path, h.title);
                }
            }
        }
        Command::Query { sql, json } => {
            // Sync first so the read-only query sees current data.
            let _ = open_synced(&root)?;
            let qconn = query::open_query_connection(&db_path(&root))?;
            let result = query::run_query(&qconn, &sql, 1000)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("{}", result.columns.join("\t"));
                for row in &result.rows {
                    let cells: Vec<String> = row
                        .iter()
                        .map(|c| match c {
                            serde_json::Value::Null => String::new(),
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        })
                        .collect();
                    println!("{}", cells.join("\t"));
                }
            }
        }
        Command::New { title } => {
            let safe = title.replace(['/', '\\'], "-");
            let rel = format!("{safe}.md");
            let abs = vault::safe_join(&root, &rel)?;
            if abs.exists() {
                bail!("note already exists: {rel}");
            }
            vault::write_note_atomic(&abs, &format!("# {title}\n\n"))?;
            let conn = db::open(&db_path(&root))?;
            index::index_single(&conn, &root, &rel)?;
            println!("{rel}");
        }
        Command::Get { path } => {
            let abs = vault::safe_join(&root, &path)?;
            print!("{}", vault::read_note(&abs)?);
        }
        Command::Export { path, html } => {
            let abs = vault::safe_join(&root, &path)?;
            let content = vault::read_note(&abs)?;
            if html {
                let parsed = parse::parse(&content);
                println!("{}", parse::to_html(&parsed.body));
            } else {
                print!("{content}");
            }
        }
        Command::Mcp => {
            mynote_core::mcp::run(&root)?;
        }
    }
    Ok(())
}
