//! Per-machine locations for derived data.
//!
//! The SQLite index is deliberately kept **out of the vault**. Vaults usually
//! live in iCloud Drive / Google Drive / Dropbox, and a live SQLite database
//! (plus its WAL/SHM files) inside a sync folder gets uploaded on every write
//! and can be corrupted when two machines open the same vault. The index is a
//! rebuildable cache, so each machine keeps its own copy locally.

use std::path::{Path, PathBuf};

/// Folder name under the OS local-data directory.
const APP_DIR_NAME: &str = "MyNote";

/// Root for MyNote's per-machine data. Honours `MYNOTE_DATA_DIR` (tests,
/// portable installs), else the OS local-data dir
/// (`~/Library/Application Support` on macOS, `%LOCALAPPDATA%` on Windows,
/// `~/.local/share` on Linux).
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("MYNOTE_DATA_DIR") {
        return PathBuf::from(dir);
    }
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(APP_DIR_NAME)
}

/// Stable key for a vault: a short hash of its canonical path, so the same
/// folder always maps to the same index regardless of how it was opened.
pub fn vault_key(root: &Path) -> String {
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let hash = blake3::hash(canonical.to_string_lossy().as_bytes()).to_hex();
    hash[..16].to_string()
}

/// Where this machine keeps the search index for the vault at `root`.
pub fn index_db_path(root: &Path) -> PathBuf {
    data_dir()
        .join("indexes")
        .join(vault_key(root))
        .join("index.db")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_key_is_stable_and_path_specific() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        assert_eq!(vault_key(a.path()), vault_key(a.path()));
        assert_ne!(vault_key(a.path()), vault_key(b.path()));
        assert_eq!(vault_key(a.path()).len(), 16);
    }

    #[test]
    fn index_lives_outside_the_vault() {
        let vault = tempfile::tempdir().unwrap();
        let index = index_db_path(vault.path());
        let canonical = vault.path().canonicalize().unwrap();
        assert!(!index.starts_with(vault.path()));
        assert!(!index.starts_with(&canonical));
        assert!(index.ends_with("index.db"));
    }
}
