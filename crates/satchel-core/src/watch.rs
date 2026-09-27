//! Vault file watcher. Reports debounced changes to `.md`/`.canvas` files so the
//! app can reconcile edits made by AI tools, other editors, or cloud sync.
//!
//! Own-writes need no special handling: the indexer is hash-incremental, so
//! re-indexing a file we just wrote (and indexed) is a cheap no-op.

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use notify::{EventKind, RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, RecommendedCache};

/// A single relevant change.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VaultEvent {
    /// Path relative to the vault root, forward-slashed.
    pub rel_path: String,
    /// True if the file was removed.
    pub removed: bool,
}

/// Keeps the watcher alive; dropping it stops watching.
pub struct WatchGuard(#[allow(dead_code)] Debouncer<RecommendedWatcher, RecommendedCache>);

const SKIP_DIRS: &[&str] = &[".satchel", ".git", ".obsidian", "node_modules", ".trash"];

fn is_relevant(rel: &str) -> bool {
    // Skip our index dir, VCS, dotdirs, and non-note files.
    for part in rel.split('/') {
        if SKIP_DIRS.contains(&part) || part.starts_with('.') {
            return false;
        }
    }
    let lower = rel.to_lowercase();
    lower.ends_with(".md") || lower.ends_with(".canvas")
}

/// Start watching `root`. `handler` is called with batches of relevant changes
/// on a background thread. Hold the returned guard for as long as you want to watch.
pub fn start<F>(root: &Path, handler: F) -> Result<WatchGuard>
where
    F: Fn(Vec<VaultEvent>) + Send + 'static,
{
    // Canonicalize so event paths (which the OS may report through resolved
    // symlinks, e.g. /var → /private/var on macOS) strip cleanly to rel paths.
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let cb_root = root.clone();
    let mut debouncer = new_debouncer(
        Duration::from_millis(400),
        None,
        move |result: DebounceEventResult| {
            let Ok(events) = result else { return };
            let mut out = Vec::new();
            let mut seen = HashSet::new();
            for event in events {
                let removed = matches!(event.kind, EventKind::Remove(_));
                for path in &event.paths {
                    let Ok(rel) = path.strip_prefix(&cb_root) else {
                        continue;
                    };
                    let rel = rel.to_string_lossy().replace('\\', "/");
                    if !is_relevant(&rel) {
                        continue;
                    }
                    if seen.insert(rel.clone()) {
                        let removed = removed || !path.exists();
                        out.push(VaultEvent { rel_path: rel, removed });
                    }
                }
            }
            if !out.is_empty() {
                handler(out);
            }
        },
    )?;
    debouncer.watch(&root, RecursiveMode::Recursive)?;
    Ok(WatchGuard(debouncer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn detects_external_markdown_change() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let _guard = start(dir.path(), move |events| {
            let _ = tx.send(events);
        })
        .unwrap();

        // Give the watcher a moment to initialize, then create a note.
        std::thread::sleep(Duration::from_millis(300));
        std::fs::write(dir.path().join("external.md"), "# Hi from outside").unwrap();

        // Collect events up to a few seconds (FSEvents has latency).
        let deadline = std::time::Instant::now() + Duration::from_secs(6);
        let mut saw = false;
        while std::time::Instant::now() < deadline {
            if let Ok(events) = rx.recv_timeout(Duration::from_millis(500)) {
                if events.iter().any(|e| e.rel_path == "external.md" && !e.removed) {
                    saw = true;
                    break;
                }
            }
        }
        assert!(saw, "watcher should report the external .md change");
    }

    #[test]
    fn ignores_index_and_dotfiles() {
        assert!(is_relevant("notes/a.md"));
        assert!(is_relevant("board.canvas"));
        assert!(!is_relevant(".satchel/index.db"));
        assert!(!is_relevant(".obsidian/config"));
        assert!(!is_relevant("notes/.hidden.md"));
        assert!(!is_relevant("readme.txt"));
    }
}
