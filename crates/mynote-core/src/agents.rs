//! Generate agent-convention docs in the vault so Claude Code / Codex use it
//! well. Never clobbers a file the user has edited.

use std::path::Path;

use anyhow::Result;

const AGENTS_MD: &str = r#"# Working in this MyNote vault

This folder is a **MyNote** vault: plain Markdown notes you (and AI tools) co-own.
Everything here is just files — safe to read and edit directly.

## Layout
- `**/*.md` — notes. Optional YAML frontmatter at the top (`---` … `---`).
- `attachments/` — images and other embedded files.
- `*.canvas` — JSONCanvas boards (https://jsoncanvas.org).
- `.mynote/` — **generated index** (SQLite) and settings. Do not edit by hand;
  it is rebuilt from the files.

## Conventions
- **Title**: frontmatter `title:`, else the first `# H1`, else the filename.
- **Links**: `[[Note Title]]`, `[[Note#Heading]]`, `[[Note|alias]]`. Embeds: `![[image.png]]`.
- **Tags**: frontmatter `tags: [a, b]` and/or inline `#tag` (nesting allowed: `#area/sub`).
- **Code**: fenced blocks. ` ```mermaid ` renders as a diagram.
- To **add a note**: create a `.md` file with a `# Title` and write Markdown. MyNote indexes it automatically.

## Driving MyNote from an agent
MyNote ships an MCP server exposing search/query/read/create over this vault.
- **Claude Code**: `claude mcp add --transport stdio mynote -- mynote mcp --vault "<this folder>"`
- **Codex**: add to `~/.codex/config.toml`:
  `[mcp_servers.mynote]` with `command = "mynote"`, `args = ["mcp", "--vault", "<this folder>"]`
- Or use the CLI directly: `mynote search "<query>" --vault "<this folder>"`,
  `mynote query "SELECT rel_path FROM notes" --vault "<this folder>"` (read-only SQL).
"#;

const CLAUDE_MD: &str = r#"# MyNote vault

See [AGENTS.md](AGENTS.md) for how this vault is structured and how to drive
MyNote (search, query, MCP). Notes are plain Markdown — edit files directly.
"#;

fn write_if_absent(path: &Path, content: &str) -> Result<()> {
    if !path.exists() {
        std::fs::write(path, content)?;
    }
    Ok(())
}

/// Write `AGENTS.md` and `CLAUDE.md` at the vault root if they don't exist.
pub fn ensure_agent_docs(root: &Path) -> Result<()> {
    write_if_absent(&root.join("AGENTS.md"), AGENTS_MD)?;
    write_if_absent(&root.join("CLAUDE.md"), CLAUDE_MD)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_docs_then_never_clobbers() {
        let dir = tempfile::tempdir().unwrap();
        ensure_agent_docs(dir.path()).unwrap();
        assert!(dir.path().join("AGENTS.md").exists());
        assert!(dir.path().join("CLAUDE.md").exists());

        // User edits AGENTS.md — a second call must not overwrite it.
        std::fs::write(dir.path().join("AGENTS.md"), "my edits").unwrap();
        ensure_agent_docs(dir.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("AGENTS.md")).unwrap(),
            "my edits"
        );
    }
}
