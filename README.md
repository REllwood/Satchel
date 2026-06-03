# MyNote

A **local-first** desktop knowledge app — think Obsidian × Evernote — for developers, solution
architects, and designers. Your notes are **plain Markdown files in a folder you choose** (including a
Google Drive / iCloud synced folder), backed by a local **SQLite** index for fast search, links, a
knowledge graph, and SQL-style queries. It's built to interoperate with AI coding tools like **Claude
Code and Codex**. Everything runs on your machine — **nothing is sent anywhere** (the only network touch
is an explicit Figma embed, if you paste one).

## Why

- **Your files, your folder.** Notes are `.md` with YAML frontmatter; attachments and canvases sit
  beside them. Point MyNote at any folder — including one inside iCloud Drive or Google Drive — and your
  notes sync via that provider. The SQLite database (`.mynote/index.db`) is a *derived cache* you can
  delete and rebuild at any time.
- **Built for AI workflows.** A bundled `mynote` CLI and a local **MCP server** let Claude Code / Codex
  search, query, read, and create notes in the same vault you edit by hand. An `AGENTS.md`/`CLAUDE.md`
  is generated in your vault explaining the conventions.
- **Private by design.** No accounts, no telemetry, no cloud backend. On-device semantic search uses a
  bundled embedding model (no download). A strict Content-Security-Policy blocks all remote content
  except Figma embeds you add.

## Features

- ✍️ **Editor** — CodeMirror 6 with Obsidian-style live preview (clean Markdown stays clean), syntax-
  highlighted code, drag/paste images, and inline **Mermaid** diagrams (sandboxed + sanitized).
- 🔗 **Links & graph** — `[[wikilinks]]` with autocomplete and ⌘-click, a backlinks/outline panel, and an
  interactive force-directed graph (global + local).
- 🔎 **Search** — instant full-text (SQLite FTS5) **and** fully-offline semantic search, plus a hybrid mode.
- 🧮 **Query view** — run **read-only** SQL over your notes index (sandboxed; SELECT-only) with saved queries.
- 🏷️ **Tags** — from `#inline` and frontmatter `tags:`, filterable from the sidebar.
- 🎨 **Canvas** — an Obsidian-compatible **JSONCanvas** board (React Flow) to arrange and link notes/cards.
- 🖼️ **Figma** — paste a share link → inline embed.
- 🤖 **AI surface** — `mynote` CLI + a local MCP server (`mynote mcp`).
- 🌗 **Themes** — graphite + indigo, light/dark following the OS, keyboard-first (⌘K search, ⌘⇧P palette).

## Prerequisites

- **[Bun](https://bun.sh)** (the project's package manager and JS runtime). The `dev`/`build`/`test`
  scripts run Vite/Vitest under Bun, so **no specific Node version is required**.
- **Rust** (stable, via [rustup](https://rustup.rs)) and the platform Tauri prerequisites
  (on macOS: Xcode Command Line Tools).

## Setup

```bash
bun install
```

## Run (development)

```bash
bun run tauri dev      # launches the desktop app with hot reload
```

## Build (release)

```bash
bun run tauri build    # produces a native bundle in src-tauri/target/release/bundle
```

## Test

```bash
cargo test --workspace   # Rust core + CLI/MCP integration tests
bun run test             # frontend (Vitest)
```

## The `mynote` CLI & MCP server

Build the CLI binary with `cargo build --release -p mynote-cli` (output: `target/release/mynote`).

```bash
mynote --vault <path> init                 # index the vault + write AGENTS.md/CLAUDE.md
mynote --vault <path> search "<query>"     # full-text search (--semantic for meaning-based)
mynote --vault <path> query "SELECT ..."   # read-only SQL over the index (--json)
mynote --vault <path> new "Title"          # create a note
mynote --vault <path> get path/to/note.md  # print a note
mynote --vault <path> export note.md --html
mynote --vault <path> mcp                  # run the MCP server over stdio
```

Connect it to your agent:

```bash
# Claude Code
claude mcp add --transport stdio mynote -- mynote mcp --vault "<your vault>"

# Codex — add to ~/.codex/config.toml
# [mcp_servers.mynote]
# command = "mynote"
# args = ["mcp", "--vault", "<your vault>"]
```

MCP tools exposed: `search_notes`, `semantic_search`, `list_notes`, `read_note`, `create_note`,
`list_tags`, `run_query` (read-only).

## Vault layout

```
<vault>/
  **/*.md            notes (Markdown + optional YAML frontmatter)
  attachments/       images and other embedded files
  *.canvas           JSONCanvas boards
  AGENTS.md          conventions for AI tools (generated)
  CLAUDE.md          pointer to AGENTS.md (generated)
  .mynote/           generated SQLite index + settings (safe to delete; rebuilt)
```

## Privacy & cloud sync

- MyNote makes **no outbound network requests** during normal use. The only exceptions are (1) a Figma
  embed you explicitly paste (loads from Figma's servers) and (2) nothing else — the embedding model is
  bundled, not downloaded.
- For cloud folders, prefer **iCloud "Keep Downloaded"** or **Google Drive "Mirror"** mode. MyNote
  tolerates not-yet-downloaded ("dataless") files and won't force-download your whole vault at startup.

## Architecture

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md). In short: one Rust core crate (`mynote-core`) holds
all logic; the Tauri desktop app, the `mynote` CLI, and the MCP server are three thin shells over it.
Files are the source of truth; SQLite is a rebuildable index.

## License

MIT.
