//! Vault file I/O: recursive note discovery, UTF-8-safe reads, atomic writes,
//! content hashing, and detection of non-materialized cloud placeholder files.
//!
//! Files are the source of truth. Writes go to a temp file in the same
//! directory and are `rename`d into place (atomic on APFS) so external readers
//! — including cloud sync and AI agents — never observe a half-written note.

use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use walkdir::WalkDir;

/// Directory names that never contain user notes.
const SKIP_DIRS: &[&str] = &[".satchel", ".git", ".obsidian", "node_modules", ".trash"];

/// A note (or placeholder) discovered during a vault scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedNote {
    /// Path relative to the vault root, forward-slashed. For a cloud
    /// placeholder this is the *real* note path (e.g. `note.md`, not
    /// `.note.md.icloud`).
    pub rel_path: String,
    pub abs_path: PathBuf,
    /// `false` when the file's contents are evicted to the cloud (iCloud
    /// `.X.icloud` placeholder) and not present locally.
    pub materialized: bool,
    /// Modification time in unix milliseconds (0 if unavailable).
    pub mtime_ms: i64,
    pub size: u64,
}

/// blake3 hex digest — the authoritative change signal (mtime is unreliable
/// under cloud sync).
pub fn content_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Read a note's text, lossily decoding invalid UTF-8 rather than failing.
pub fn read_note(path: &Path) -> io::Result<String> {
    let bytes = fs::read(path)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Write a note atomically (temp file in the same directory + rename).
pub fn write_note_atomic(path: &Path, content: &str) -> io::Result<()> {
    write_bytes_atomic(path, content.as_bytes())
}

/// Write arbitrary bytes atomically (temp file in the same directory + rename).
pub fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".satchel-tmp-")
        .tempfile_in(parent)?;
    tmp.write_all(bytes)?;
    tmp.flush()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Join a vault-relative path to the root, rejecting absolute paths and any
/// `..` traversal so a path can never escape the vault. Used wherever an
/// untrusted (UI/CLI/MCP) relative path is resolved.
pub fn safe_join(root: &Path, rel: &str) -> io::Result<PathBuf> {
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "absolute paths are not allowed",
        ));
    }
    for component in rel_path.components() {
        match component {
            Component::ParentDir | Component::Prefix(_) | Component::RootDir => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "path escapes the vault",
                ));
            }
            _ => {}
        }
    }
    Ok(root.join(rel_path))
}

/// If `name` is an iCloud placeholder (`.<real>.icloud`), return the real name.
fn icloud_real_name(name: &str) -> Option<&str> {
    name.strip_prefix('.')?.strip_suffix(".icloud")
}

fn is_markdown(name: &str) -> bool {
    name.rsplit('.')
        .next()
        .map(|ext| ext.eq_ignore_ascii_case("md"))
        .unwrap_or(false)
}

/// Modification time in unix milliseconds (0 if unavailable).
pub fn mtime_ms(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// True when a file's contents are evicted to the cloud (iCloud Drive on
/// macOS 14+, Google Drive "stream" mode and other File Provider folders).
/// Checking the `SF_DATALESS` flag via `stat` never triggers a download —
/// unlike reading the file, which makes macOS fetch it.
#[cfg(target_os = "macos")]
pub fn is_dataless(meta: &fs::Metadata) -> bool {
    use std::os::macos::fs::MetadataExt;
    const SF_DATALESS: u32 = 0x4000_0000;
    meta.st_flags() & SF_DATALESS != 0
}

/// Windows cloud placeholders (OneDrive Files On-Demand, iCloud for Windows)
/// are recalled from the cloud on first read.
#[cfg(windows)]
pub fn is_dataless(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_OFFLINE: u32 = 0x0000_1000;
    const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
    meta.file_attributes() & (FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS) != 0
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn is_dataless(_meta: &fs::Metadata) -> bool {
    false
}

/// Let this process download a cloud-only file when it reads one. macOS
/// decides per process; apps launched from the Dock allow it, but processes
/// started by agents or launchd may not and would get EDEADLK instead. The
/// scanner never reads dataless files, so this only affects explicit reads.
pub fn allow_cloud_downloads() {
    #[cfg(target_os = "macos")]
    {
        extern "C" {
            fn setiopolicy_np(iotype: i32, scope: i32, policy: i32) -> i32;
        }
        const IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES: i32 = 3;
        const IOPOL_SCOPE_PROCESS: i32 = 0;
        const IOPOL_MATERIALIZE_DATALESS_FILES_ON: i32 = 2;
        // SAFETY: plain libSystem call with constant arguments; it only
        // changes this process's I/O policy.
        unsafe {
            setiopolicy_np(
                IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES,
                IOPOL_SCOPE_PROCESS,
                IOPOL_MATERIALIZE_DATALESS_FILES_ON,
            );
        }
    }
}

fn rel_path_str(root: &Path, abs: &Path) -> Option<String> {
    let rel = abs.strip_prefix(root).ok()?;
    Some(rel.to_string_lossy().replace('\\', "/"))
}

/// Recursively scan a vault for Markdown notes, skipping config/VCS folders and
/// dotfiles. Cloud placeholders are reported as non-materialized notes under
/// their real path; a materialized file always wins over a placeholder.
pub fn scan_vault(root: &Path) -> Vec<ScannedNote> {
    use std::collections::BTreeMap;
    let mut found: BTreeMap<String, ScannedNote> = BTreeMap::new();

    let walker = WalkDir::new(root).follow_links(false).into_iter();
    for entry in walker.filter_entry(|e| {
        if e.file_type().is_dir() {
            let name = e.file_name().to_string_lossy();
            // Always allow the root itself; skip known dirs and dotdirs below it.
            e.depth() == 0 || (!SKIP_DIRS.contains(&name.as_ref()) && !name.starts_with('.'))
        } else {
            true
        }
    }) {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() {
            continue;
        }
        let abs = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();

        // Resolve the logical note path + whether it's materialized.
        let (logical_name, materialized) = match icloud_real_name(&name) {
            Some(real) if is_markdown(real) => (real.to_string(), false),
            _ if is_markdown(&name) && !name.starts_with('.') => (name.clone(), true),
            _ => continue,
        };

        let logical_abs = abs.with_file_name(&logical_name);
        let Some(rel) = rel_path_str(root, &logical_abs) else {
            continue;
        };

        let meta = entry.metadata().ok();
        // Modern cloud placeholders keep the real name but have no local data.
        let materialized = materialized && !meta.as_ref().is_some_and(is_dataless);
        let note = ScannedNote {
            rel_path: rel.clone(),
            abs_path: logical_abs,
            materialized,
            mtime_ms: meta.as_ref().map(mtime_ms).unwrap_or(0),
            // Dataless files still report their logical size, which lets the
            // indexer recognise them as unchanged without downloading.
            size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
        };

        // Materialized wins over a placeholder for the same logical path.
        found
            .entry(rel)
            .and_modify(|existing| {
                if note.materialized && !existing.materialized {
                    *existing = note.clone();
                }
            })
            .or_insert(note);
    }

    found.into_values().collect()
}

/// List vault-relative paths with the given extension (e.g. "canvas"),
/// skipping config/dot directories. Same traversal rules as `scan_vault`.
pub fn list_by_ext(root: &Path, ext: &str) -> Vec<String> {
    let mut out = Vec::new();
    let walker = WalkDir::new(root).follow_links(false).into_iter();
    for entry in walker.filter_entry(|e| {
        if e.file_type().is_dir() {
            let name = e.file_name().to_string_lossy();
            e.depth() == 0 || (!SKIP_DIRS.contains(&name.as_ref()) && !name.starts_with('.'))
        } else {
            true
        }
    }) {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let matches = name
            .rsplit('.')
            .next()
            .map(|e| e.eq_ignore_ascii_case(ext))
            .unwrap_or(false);
        if matches {
            if let Some(rel) = rel_path_str(root, entry.path()) {
                out.push(rel);
            }
        }
    }
    out.sort();
    out
}

/// List the vault's folders ("notebooks") as relative paths, including empty
/// ones. Skips hidden/config dirs and the top-level `attachments` folder.
pub fn list_dirs(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let walker = WalkDir::new(root).follow_links(false).min_depth(1).into_iter();
    for entry in walker.filter_entry(|e| {
        let name = e.file_name().to_string_lossy();
        !(e.file_type().is_dir()
            && (SKIP_DIRS.contains(&name.as_ref())
                || name.starts_with('.')
                || (e.depth() == 1 && name == "attachments")))
    }) {
        let Ok(entry) = entry else { continue };
        if entry.file_type().is_dir() {
            if let Some(rel) = rel_path_str(root, entry.path()) {
                out.push(rel);
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, content: &str) {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn scan_finds_md_and_skips_config_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("a.md"), "# A");
        write(&root.join("sub/b.md"), "# B");
        write(&root.join("notes.txt"), "ignored");
        write(&root.join(".satchel/index.db"), "db");
        write(&root.join(".satchel/c.md"), "# hidden, ignored");
        write(&root.join("node_modules/d.md"), "# dep, ignored");

        let mut paths: Vec<_> = scan_vault(root).into_iter().map(|n| n.rel_path).collect();
        paths.sort();
        assert_eq!(paths, vec!["a.md".to_string(), "sub/b.md".to_string()]);
    }

    #[test]
    fn atomic_write_replaces_content_and_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        write_note_atomic(&path, "first").unwrap();
        assert_eq!(read_note(&path).unwrap(), "first");
        write_note_atomic(&path, "second").unwrap();
        assert_eq!(read_note(&path).unwrap(), "second");
    }

    #[test]
    fn icloud_placeholder_is_reported_as_non_materialized() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // An evicted note shows up only as `.note.md.icloud`.
        write(&root.join(".note.md.icloud"), "");
        let notes = scan_vault(root);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].rel_path, "note.md");
        assert!(!notes[0].materialized);
    }

    #[test]
    fn materialized_file_wins_over_placeholder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("note.md"), "real content");
        write(&root.join(".note.md.icloud"), "");
        let notes = scan_vault(root);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].materialized);
    }

    #[test]
    fn content_hash_is_stable_and_distinct() {
        assert_eq!(content_hash(b"abc"), content_hash(b"abc"));
        assert_ne!(content_hash(b"abc"), content_hash(b"abd"));
    }

    #[test]
    fn list_dirs_includes_empty_folders_but_not_hidden_or_attachments() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("Work/Projects")).unwrap();
        fs::create_dir_all(root.join("Personal")).unwrap();
        fs::create_dir_all(root.join("attachments")).unwrap();
        fs::create_dir_all(root.join(".satchel")).unwrap();
        fs::create_dir_all(root.join(".obsidian")).unwrap();
        assert_eq!(
            list_dirs(root),
            vec!["Personal".to_string(), "Work".into(), "Work/Projects".into()]
        );
    }

    #[test]
    fn regular_files_are_not_dataless() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.md");
        write(&path, "hi");
        assert!(!is_dataless(&fs::metadata(&path).unwrap()));
    }

    #[test]
    fn safe_join_blocks_traversal_and_absolute() {
        let root = Path::new("/vault");
        assert!(safe_join(root, "notes/a.md").is_ok());
        assert!(safe_join(root, "attachments/img.png").is_ok());
        assert!(safe_join(root, "../etc/passwd").is_err());
        assert!(safe_join(root, "a/../../b").is_err());
        assert!(safe_join(root, "/etc/passwd").is_err());
    }
}
