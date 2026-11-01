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
///
/// The data is flushed to disk before the rename, so a crash leaves either the
/// old or the new contents, never an empty file. The file keeps its
/// permissions and Finder tags, and a symlinked note is written through to the
/// file it points at instead of being replaced by a plain file.
pub fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let target = match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
        }
        _ => path.to_path_buf(),
    };
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let existing = fs::metadata(&target).ok();
    let mut tmp = temp_file_like(parent, existing.as_ref())?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    #[cfg(target_os = "macos")]
    if existing.is_some() {
        copy_finder_metadata(&target, tmp.path());
    }
    tmp.persist(&target).map_err(|e| e.error)?;
    sync_dir(parent);
    Ok(())
}

/// Create a new file with `bytes`, failing with `AlreadyExists` instead of
/// replacing a file that is already there (even one created a moment ago).
pub fn create_file_exclusive(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut tmp = temp_file_like(parent, None)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist_noclobber(path).map_err(|e| e.error)?;
    sync_dir(parent);
    Ok(())
}

/// A temp file in `dir` with the permissions of `like` (the file it will
/// replace), or the usual `rw-r--r--` for a new file rather than tempfile's
/// private default, which would hide notes from other accounts and tools.
fn temp_file_like(dir: &Path, like: Option<&fs::Metadata>) -> io::Result<tempfile::NamedTempFile> {
    let mut builder = tempfile::Builder::new();
    builder.prefix(".satchel-tmp-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = like.map(|m| m.permissions().mode() & 0o7777).unwrap_or(0o644);
        builder.permissions(fs::Permissions::from_mode(mode));
    }
    #[cfg(not(unix))]
    let _ = like;
    builder.tempfile_in(dir)
}

/// Flush a directory entry change (the rename) to disk. Best effort.
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Copy Finder tags and colour labels from `from` to `to`. Best effort.
#[cfg(target_os = "macos")]
fn copy_finder_metadata(from: &Path, to: &Path) {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let (Ok(from), Ok(to)) = (
        CString::new(from.as_os_str().as_bytes()),
        CString::new(to.as_os_str().as_bytes()),
    ) else {
        return;
    };
    for name in ["com.apple.metadata:_kMDItemUserTags", "com.apple.FinderInfo"] {
        let name = CString::new(name).unwrap();
        let mut buf = vec![0u8; 4096];
        // SAFETY: valid NUL-terminated paths and names, and a buffer of the
        // stated length. A missing attribute just returns -1.
        unsafe {
            let len = libc::getxattr(
                from.as_ptr(),
                name.as_ptr(),
                buf.as_mut_ptr().cast(),
                buf.len(),
                0,
                0,
            );
            if len > 0 {
                libc::setxattr(to.as_ptr(), name.as_ptr(), buf.as_ptr().cast(), len as usize, 0, 0);
            }
        }
    }
}

/// A file name for a note titled `title`: characters that are not allowed in
/// file names on macOS, Windows or Linux become `-`, and leading dots (which
/// would hide the note) and trailing dots and spaces (which Windows drops)
/// are removed. Never empty.
pub fn note_file_name(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let trimmed = cleaned
        .trim()
        .trim_start_matches('.')
        .trim_end_matches(['.', ' '])
        .trim();
    let stem = if trimmed.is_empty() { "Untitled" } else { trimmed };
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
        "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(stem)) {
        format!("{stem} note")
    } else {
        stem.to_string()
    }
}

/// Join a vault-relative path to the root, rejecting absolute paths, any
/// `..` traversal, and symlinks that lead outside the vault, so a path can
/// never escape it. Used wherever an untrusted (UI/CLI/MCP) relative path is
/// resolved.
pub fn safe_join(root: &Path, rel: &str) -> io::Result<PathBuf> {
    let joined = join_lexically(root, rel)?;
    ensure_inside(root, &joined)?;
    Ok(joined)
}

/// Check that `path`, with symlinks resolved, is inside `root`. Parts of the
/// path that do not exist yet cannot be links, so the deepest existing
/// ancestor is what gets checked.
fn ensure_inside(root: &Path, path: &Path) -> io::Result<()> {
    let Ok(real_root) = root.canonicalize() else {
        return Ok(()); // no vault folder, so nothing under it exists either
    };
    let mut probe = path;
    loop {
        if let Ok(real) = probe.canonicalize() {
            if real.starts_with(&real_root) {
                return Ok(());
            }
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "path leads outside the vault",
            ));
        }
        match probe.parent() {
            Some(parent) if parent.starts_with(root) => probe = parent,
            _ => return Ok(()),
        }
    }
}

fn join_lexically(root: &Path, rel: &str) -> io::Result<PathBuf> {
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

/// True for a Markdown note path or file name (`.md`, any case).
pub fn is_markdown(name: &str) -> bool {
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

/// True when `rel` is inside a folder the scanner skips (config, VCS, any dot
/// folder) or is itself a dotfile, so it is never treated as a note.
pub fn is_hidden_path(rel: &str) -> bool {
    rel.split('/')
        .any(|part| SKIP_DIRS.contains(&part) || part.starts_with('.'))
}

/// The macOS 13 iCloud placeholder (`.<name>.icloud`) standing in for `abs`,
/// if there is one. Newer macOS versions keep the real name instead.
pub fn icloud_placeholder(abs: &Path) -> Option<PathBuf> {
    let name = abs.file_name()?.to_str()?;
    let placeholder = abs.with_file_name(format!(".{name}.icloud"));
    placeholder.is_file().then_some(placeholder)
}

/// Make sure `abs` has its contents locally. A macOS 13 iCloud placeholder is
/// downloaded (which can take a while), waiting up to `timeout`. On newer
/// systems, reading a cloud-only file downloads it, so there is nothing to do.
pub fn ensure_local(abs: &Path, timeout: std::time::Duration) -> io::Result<()> {
    if abs.exists() || icloud_placeholder(abs).is_none() {
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("/usr/bin/brctl")
            .arg("download")
            .arg(abs)
            .status()?;
        if !status.success() {
            return Err(io::Error::other("iCloud Drive could not download the note"));
        }
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if abs.is_file() {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "still downloading from iCloud Drive, try again in a moment",
        ))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = timeout;
        Ok(())
    }
}

/// True when `abs` exists under exactly this name. On case-insensitive disks
/// `Notes.md` also opens `notes.md`, so after a case-only or Unicode-form
/// rename the old spelling still "exists" and must be checked against the
/// folder listing.
pub fn exists_exactly(abs: &Path) -> bool {
    if !abs.is_file() {
        return false;
    }
    if cfg!(not(any(target_os = "macos", windows))) {
        return true;
    }
    let (Some(parent), Some(name)) = (abs.parent(), abs.file_name()) else {
        return true;
    };
    match fs::read_dir(parent) {
        Ok(entries) => entries.flatten().any(|e| e.file_name() == name),
        Err(_) => true,
    }
}

/// Find the file an embed or link in `from_note` points at, the way Obsidian
/// does: next to the note first, then from the vault root, then (for a bare
/// file name) anywhere in the vault, preferring the shortest path.
pub fn resolve_attachment(root: &Path, from_note: Option<&str>, target: &str) -> io::Result<PathBuf> {
    let target = target.trim().trim_start_matches('/');
    let not_found = || io::Error::new(io::ErrorKind::NotFound, format!("{target} was not found"));
    if target.is_empty() {
        return Err(not_found());
    }
    let mut candidates = Vec::new();
    if let Some(dir) = from_note.and_then(|n| n.rsplit_once('/')).map(|(dir, _)| dir) {
        if let Some(rel) = normalize_rel(&format!("{dir}/{target}")) {
            candidates.push(rel);
        }
    }
    if let Some(rel) = normalize_rel(target) {
        candidates.push(rel);
    }
    for rel in candidates {
        if let Ok(abs) = safe_join(root, &rel) {
            if abs.is_file() {
                return Ok(abs);
            }
        }
    }
    if target.contains('/') {
        return Err(not_found());
    }
    let mut best: Option<PathBuf> = None;
    let walker = WalkDir::new(root).follow_links(false).into_iter();
    for entry in walker.filter_entry(|e| {
        e.depth() == 0 || !(e.file_type().is_dir() && is_hidden_path(&e.file_name().to_string_lossy()))
    }) {
        let Ok(entry) = entry else { continue };
        if entry.file_type().is_file() && entry.file_name().to_string_lossy() == target {
            let shorter = best
                .as_ref()
                .is_none_or(|b| entry.path().as_os_str().len() < b.as_os_str().len());
            if shorter {
                best = Some(entry.into_path());
            }
        }
    }
    best.ok_or_else(not_found)
}

/// Resolve `.` and `..` in a vault-relative path. `None` if it climbs above
/// the vault root.
fn normalize_rel(rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for part in rel.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn rel_path_str(root: &Path, abs: &Path) -> Option<String> {
    let rel = abs.strip_prefix(root).ok()?;
    Some(rel.to_string_lossy().replace('\\', "/"))
}

/// The result of a full vault scan.
#[derive(Debug, Default)]
pub struct VaultScan {
    pub notes: Vec<ScannedNote>,
    /// Folders (vault-relative) that could not be listed. Their notes are
    /// missing from `notes` but must not be treated as deleted.
    pub unreadable: Vec<String>,
}

/// Recursively scan a vault for Markdown notes, skipping config/VCS folders and
/// dotfiles. Cloud placeholders are reported as non-materialized notes under
/// their real path; a materialized file always wins over a placeholder.
pub fn scan_vault(root: &Path) -> Vec<ScannedNote> {
    scan_vault_report(root).map(|s| s.notes).unwrap_or_default()
}

/// Like [`scan_vault`], but fails if the vault folder itself is missing or
/// unreadable (for example a cloud drive that is not mounted yet), and reports
/// subfolders it could not list, so callers never mistake an unreadable folder
/// for an empty one.
pub fn scan_vault_report(root: &Path) -> io::Result<VaultScan> {
    use std::collections::BTreeMap;
    fs::read_dir(root)?;
    let mut found: BTreeMap<String, ScannedNote> = BTreeMap::new();
    let mut unreadable = Vec::new();

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
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                match err.path().and_then(|p| rel_path_str(root, p)) {
                    Some(rel) if !rel.is_empty() => unreadable.push(rel),
                    _ => return Err(err.into()),
                }
                continue;
            }
        };
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

    Ok(VaultScan {
        notes: found.into_values().collect(),
        unreadable,
    })
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
    fn note_file_names_are_safe_everywhere() {
        assert_eq!(note_file_name("Plan: Q3/Q4?"), "Plan- Q3-Q4-");
        assert_eq!(note_file_name(".NET tips"), "NET tips");
        assert_eq!(note_file_name("  ...  "), "Untitled");
        assert_eq!(note_file_name("trailing. "), "trailing");
        assert_eq!(note_file_name("con"), "con note");
    }

    #[cfg(unix)]
    #[test]
    fn saving_keeps_permissions_and_writes_through_symlinks() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let note = dir.path().join("a.md");
        write(&note, "old");
        fs::set_permissions(&note, fs::Permissions::from_mode(0o640)).unwrap();
        write_note_atomic(&note, "new").unwrap();
        assert_eq!(fs::metadata(&note).unwrap().permissions().mode() & 0o777, 0o640);

        let fresh = dir.path().join("b.md");
        write_note_atomic(&fresh, "x").unwrap();
        assert_eq!(fs::metadata(&fresh).unwrap().permissions().mode() & 0o777, 0o644);

        let link = dir.path().join("link.md");
        std::os::unix::fs::symlink(&note, &link).unwrap();
        write_note_atomic(&link, "through").unwrap();
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(&note).unwrap(), "through");
    }

    #[test]
    fn exclusive_create_never_replaces_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.md");
        create_file_exclusive(&path, b"one").unwrap();
        let err = create_file_exclusive(&path, b"two").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&path).unwrap(), "one");
    }

    #[cfg(unix)]
    #[test]
    fn safe_join_refuses_symlinks_out_of_the_vault() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        write(&outside.path().join("secret.md"), "x");
        std::os::unix::fs::symlink(outside.path(), dir.path().join("linked")).unwrap();
        assert!(safe_join(dir.path(), "linked/secret.md").is_err());
        assert!(safe_join(dir.path(), "linked/new.md").is_err());
        assert!(safe_join(dir.path(), "inside/new.md").is_ok());
    }

    #[test]
    fn attachments_resolve_like_obsidian() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("Projects/pic.png"), "near");
        write(&root.join("pic.png"), "root");
        write(&root.join("assets/deep/only.png"), "deep");
        write(&root.join("assets/shared.png"), "shallow");
        write(&root.join("other/very/deep/shared.png"), "deeper");
        let read = |from: Option<&str>, target: &str| {
            fs::read_to_string(resolve_attachment(root, from, target).unwrap()).unwrap()
        };
        assert_eq!(read(Some("Projects/x.md"), "pic.png"), "near");
        assert_eq!(read(Some("x.md"), "pic.png"), "root");
        assert_eq!(read(Some("Projects/x.md"), "../pic.png"), "root");
        assert_eq!(read(None, "only.png"), "deep");
        assert_eq!(read(None, "shared.png"), "shallow");
        assert!(resolve_attachment(root, None, "missing.png").is_err());
        assert!(resolve_attachment(root, Some("x.md"), "../../etc/passwd").is_err());
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
