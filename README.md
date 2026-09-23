# MyNote

An open-source, **local-first** notes app — think Evernote × Obsidian — for developers, solution
architects and designers. Your notes are **plain Markdown files in a folder you choose**. Put that
folder in **iCloud Drive or Google Drive** (or Dropbox / OneDrive) and your notes sync to your other
devices through that service. MyNote has no account, no server and no telemetry: it never uploads
anything itself.

## How cloud sync works

On first launch MyNote looks for the sync folders already on your computer and offers them:

| Service | Where MyNote puts your notes (macOS) |
|---|---|
| iCloud Drive | `~/Library/Mobile Documents/com~apple~CloudDocs/MyNote` |
| Google Drive | `~/Library/CloudStorage/GoogleDrive-<you>/My Drive/MyNote` |
| Dropbox / OneDrive / Box | `~/Library/CloudStorage/<service>/MyNote` |
| This computer only | `~/MyNote` |

On Windows it finds `iCloudDrive`, `G:\My Drive`, Dropbox and OneDrive the same way. You can also
choose any other folder, or later go to **Settings → Notes location → Change…**.

- **Google Drive** needs [Google Drive for desktop](https://www.google.com/drive/download/). Sign
  in, and the Google Drive folder shows up in MyNote. **iCloud Drive** needs to be turned on in
  System Settings → Apple Account → iCloud.
- MyNote's search database is kept **per computer** in its app-data folder, not in your notes folder.
  Only your notes and attachments sync, and two computers never fight over one database file.
- Cloud-only files (evicted by "Optimize Mac Storage", or streamed by Google Drive) stay cloud-only
  until you open them. MyNote doesn't download your whole library at startup. Notes it has seen
  before stay searchable, and new cloud-only notes are indexed once they're downloaded. For fully
  offline search, mark the folder **Keep Downloaded** (iCloud) or **Available offline** (Google Drive).
- Deleting a note or folder moves it to the system **Trash**, so you can get it back.

## Features

- ✍️ **Editor** — CodeMirror 6 with Obsidian-style live preview, syntax-highlighted code,
  paste/drag-in images, and inline **Mermaid** diagrams (sandboxed and sanitized).
- 🗂️ **Folders** — create, rename, move and delete folders; move notes between them.
- 🔗 **Links & graph** — `[[wikilinks]]` with autocomplete and ⌘-click, backlinks, outline, and an
  interactive graph.
- 🔎 **Search** — instant full-text search (SQLite FTS5), fully offline **semantic** search using a
  bundled embedding model that keeps itself up to date, and a hybrid mode.
- 🧮 **Query view** — read-only SQL over your notes index.
- 🏷️ **Tags** — `#inline` and frontmatter `tags:`, filterable from the sidebar.
- 🎨 **Canvas** — Obsidian-compatible **JSONCanvas** boards.
- 🖼️ **Figma** — paste a share link to embed it inline.
- 🤖 **Claude Code & Codex** — the app is also an MCP server (see below).
- 🌗 Light/dark themes that follow the OS. Keyboard-first: ⌘K search, ⌘⇧P commands, ⌘N new note.

## Install

Download the latest build from
[Releases](https://github.com/REllwood/Satchel/releases): a `.dmg` for macOS (Apple silicon,
macOS 13.4+), `.msi`/`.exe` for Windows, and `.AppImage`/`.deb`/`.rpm` for Linux.

Release builds are not notarized by Apple yet, so macOS blocks the first launch. After copying
MyNote to Applications, either:

- open **System Settings → Privacy & Security** and click **Open Anyway**, or
- run `xattr -dr com.apple.quarantine /Applications/MyNote.app` in Terminal.

## Use with Claude Code and Codex

The MyNote app includes an MCP server with these tools: `search_notes`, `semantic_search`,
`list_notes`, `read_note`, `create_note`, `list_tags` and `run_query` (read-only). **Settings → AI
tools** copies the exact command for your computer. It looks like this:

```bash
claude mcp add --transport stdio mynote -- "/Applications/MyNote.app/Contents/MacOS/mynote-app" mcp --vault "<your notes folder>"
```

For Codex, add this to `~/.codex/config.toml`:

```toml
[mcp_servers.mynote]
command = "/Applications/MyNote.app/Contents/MacOS/mynote-app"
args = ["mcp", "--vault", "<your notes folder>"]
```

**Settings → Agent guide** adds an `AGENTS.md` and a `CLAUDE.md` to your notes folder, explaining
its conventions to AI coding tools.

### Command-line tool (optional)

```bash
cargo install --path crates/mynote-cli     # installs `mynote`

mynote --vault <folder> search "<query>"   # full-text search (--semantic for meaning-based)
mynote --vault <folder> query "SELECT ..." # read-only SQL over the index (--json)
mynote --vault <folder> new "Title"        # create a note
mynote --vault <folder> get note.md        # print a note
mynote --vault <folder> export note.md --html
mynote --vault <folder> mcp                # MCP server over stdio
```

## Notes folder layout

```
<notes folder>/
  **/*.md        notes (Markdown + optional YAML frontmatter)
  attachments/   images and other embedded files
  *.canvas       JSONCanvas boards
  AGENTS.md      conventions for AI tools (optional, from Settings)
```

## Privacy

MyNote makes **no network requests** in normal use. The embedding model ships inside the app. The
only exception is a Figma embed you paste, which loads from Figma. A strict Content-Security-Policy
blocks all other remote content.

## Development

Prerequisites: [Bun](https://bun.sh), stable [Rust](https://rustup.rs), and the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS.

```bash
bun install
bun run tauri dev          # desktop app with hot reload
bun run tauri build        # native bundles in target/release/bundle
cargo test --workspace     # Rust core + CLI/MCP integration tests
bun run test               # frontend tests (Vitest)
```

The code is laid out as one Rust core crate, `crates/mynote-core`, which holds all the logic (vault
I/O, parsing, SQLite/FTS5 + sqlite-vec index, embeddings, MCP). Three thin shells sit on top of it:
the Tauri desktop app (`src-tauri`, with the React UI in `src`), the `mynote` CLI
(`crates/mynote-cli`), and the MCP server. The files are the source of truth, and the index can be
rebuilt from them at any time.

Tagging a commit `v*` runs the release workflow, which builds installers for every platform into
a draft GitHub release.

## License

[MIT](LICENSE)
