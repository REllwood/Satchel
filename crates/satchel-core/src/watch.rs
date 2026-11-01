//! Vault file watcher. Reports debounced changes to `.md`/`.canvas` files so the
//! app can reconcile edits made by AI tools, other editors, or cloud sync.
//!
//! Own-writes need no special handling: the indexer is hash-incremental, so
//! re-indexing a file we just wrote (and indexed) is a cheap no-op.

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use notify::event::{CreateKind, ModifyKind, RemoveKind};
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

/// One debounced batch of changes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WatchBatch {
    pub events: Vec<VaultEvent>,
    /// True when per-file events cannot describe what happened: a folder was
    /// renamed, moved in or out, or deleted (the OS reports only the folder),
    /// or the OS dropped events. The receiver should rescan the whole vault.
    pub rescan: bool,
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

/// The note an iCloud placeholder (`.<note>.icloud`, macOS 13) stands for.
fn placeholder_note(rel: &str) -> Option<String> {
    let (dir, name) = match rel.rsplit_once('/') {
        Some((dir, name)) => (Some(dir), name),
        None => (None, rel),
    };
    let real = name.strip_prefix('.')?.strip_suffix(".icloud")?;
    let rel = match dir {
        Some(dir) => format!("{dir}/{real}"),
        None => real.to_string(),
    };
    is_relevant(&rel).then_some(rel)
}

/// True for an event about a folder the vault cares about, whose notes the
/// OS does not report one by one.
fn is_folder_change(kind: &EventKind, rel: &str, path: &Path) -> bool {
    if rel.split('/').any(|part| SKIP_DIRS.contains(&part) || part.starts_with('.')) {
        return false;
    }
    match kind {
        EventKind::Create(CreateKind::Folder) | EventKind::Remove(RemoveKind::Folder) => true,
        // A rename reports the old and new names. The new one is a folder on
        // disk, and the old one, now gone, has no file extension.
        EventKind::Modify(ModifyKind::Name(_)) => {
            path.is_dir() || (!path.exists() && !rel.rsplit('/').next().unwrap_or(rel).contains('.'))
        }
        _ => path.is_dir() && !matches!(kind, EventKind::Modify(_) | EventKind::Access(_)),
    }
}

/// Start watching `root`. `handler` is called with batches of relevant changes
/// on a background thread. Hold the returned guard for as long as you want to watch.
pub fn start<F>(root: &Path, handler: F) -> Result<WatchGuard>
where
    F: Fn(WatchBatch) + Send + 'static,
{
    // Canonicalize so event paths (which the OS may report through resolved
    // symlinks, e.g. /var → /private/var on macOS) strip cleanly to rel paths.
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let cb_root = root.clone();
    let mut debouncer = new_debouncer(
        Duration::from_millis(400),
        None,
        move |result: DebounceEventResult| {
            let mut batch = WatchBatch::default();
            // An error means events were lost.
            let events = result.unwrap_or_else(|_| {
                batch.rescan = true;
                Vec::new()
            });
            let mut seen = HashSet::new();
            for event in events {
                if event.need_rescan() {
                    batch.rescan = true;
                    continue;
                }
                for path in &event.paths {
                    let Ok(rel) = path.strip_prefix(&cb_root) else {
                        continue;
                    };
                    let rel = rel.to_string_lossy().replace('\\', "/");
                    if rel.is_empty() {
                        continue;
                    }
                    if let Some(note) = placeholder_note(&rel) {
                        // The note was evicted to (or restored from) iCloud.
                        if seen.insert(note.clone()) {
                            batch.events.push(VaultEvent { rel_path: note, removed: false });
                        }
                        continue;
                    }
                    if !is_relevant(&rel) {
                        batch.rescan |= is_folder_change(&event.kind, &rel, path);
                        continue;
                    }
                    if seen.insert(rel.clone()) {
                        // Judge by what is on disk now: an editor that saves
                        // by replacing the file reports a remove, then a create.
                        let removed = !path.exists();
                        batch.events.push(VaultEvent { rel_path: rel, removed });
                    }
                }
            }
            if !batch.events.is_empty() || batch.rescan {
                handler(batch);
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
        let _guard = start(dir.path(), move |batch| {
            let _ = tx.send(batch.events);
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
    fn folder_rename_asks_for_a_rescan() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Projects")).unwrap();
        std::fs::write(dir.path().join("Projects/a.md"), "# A").unwrap();
        let (tx, rx) = mpsc::channel();
        let _guard = start(dir.path(), move |batch| {
            let _ = tx.send(batch);
        })
        .unwrap();
        std::thread::sleep(Duration::from_millis(300));
        std::fs::rename(dir.path().join("Projects"), dir.path().join("Work")).unwrap();

        let deadline = std::time::Instant::now() + Duration::from_secs(6);
        let mut rescan = false;
        while std::time::Instant::now() < deadline && !rescan {
            if let Ok(batch) = rx.recv_timeout(Duration::from_millis(500)) {
                rescan = batch.rescan;
            }
        }
        assert!(rescan, "a folder rename must trigger a rescan");
    }

    #[test]
    fn icloud_placeholders_map_to_their_note() {
        assert_eq!(placeholder_note("sub/.Plan.md.icloud").as_deref(), Some("sub/Plan.md"));
        assert_eq!(placeholder_note(".a.md.icloud").as_deref(), Some("a.md"));
        assert_eq!(placeholder_note(".pic.png.icloud"), None);
        assert_eq!(placeholder_note(".obsidian/.x.md.icloud"), None);
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
