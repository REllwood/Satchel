//! Find cloud-synced folders (iCloud Drive, Google Drive, Dropbox, OneDrive,
//! Box) so the app can offer "keep my notes in iCloud / Google Drive" in one
//! click. Satchel never talks to a cloud API: the provider's own desktop client
//! syncs the folder, so notes stay plain files the user fully owns.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Name of the folder created inside the chosen location.
pub const VAULT_FOLDER_NAME: &str = "Satchel";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    #[serde(rename = "icloud")]
    ICloud,
    GoogleDrive,
    Dropbox,
    OneDrive,
    Box,
    Local,
}

impl Provider {
    pub fn display_name(self) -> &'static str {
        match self {
            Provider::ICloud => "iCloud Drive",
            Provider::GoogleDrive => "Google Drive",
            Provider::Dropbox => "Dropbox",
            Provider::OneDrive => "OneDrive",
            Provider::Box => "Box",
            Provider::Local => "This computer",
        }
    }
}

/// A place a vault can be created.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncLocation {
    pub provider: Provider,
    /// Human label, e.g. "Google Drive (me@gmail.com)".
    pub label: String,
    /// The vault folder Satchel would use (`<location>/Satchel`).
    pub vault_path: String,
    /// True if that folder already exists (open instead of create).
    pub vault_exists: bool,
}

fn location(provider: Provider, label: String, base: &Path) -> SyncLocation {
    let vault = base.join(VAULT_FOLDER_NAME);
    SyncLocation {
        provider,
        label,
        vault_exists: vault.is_dir(),
        vault_path: vault.to_string_lossy().into_owned(),
    }
}

/// Detect sync locations on this machine, cloud providers first, then a
/// local-only option. `home` is injectable for tests.
pub fn detect_locations_in(home: &Path) -> Vec<SyncLocation> {
    let mut out = Vec::new();

    // macOS: iCloud Drive + File Provider folders under ~/Library/CloudStorage.
    let icloud = home.join("Library/Mobile Documents/com~apple~CloudDocs");
    if icloud.is_dir() {
        out.push(location(Provider::ICloud, "iCloud Drive".into(), &icloud));
    }
    if let Ok(entries) = std::fs::read_dir(home.join("Library/CloudStorage")) {
        let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        entries.sort();
        for dir in entries.into_iter().filter(|p| p.is_dir()) {
            let name = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if let Some(account) = name.strip_prefix("GoogleDrive-") {
                let my_drive = dir.join("My Drive");
                let base = if my_drive.is_dir() { my_drive } else { dir.clone() };
                let label = format!("Google Drive ({account})");
                out.push(location(Provider::GoogleDrive, label, &base));
            } else if name.starts_with("Dropbox") {
                out.push(location(Provider::Dropbox, "Dropbox".into(), &dir));
            } else if let Some(rest) = name.strip_prefix("OneDrive") {
                let suffix = rest.trim_start_matches('-');
                let label = if suffix.is_empty() {
                    "OneDrive".to_string()
                } else {
                    format!("OneDrive ({suffix})")
                };
                out.push(location(Provider::OneDrive, label, &dir));
            } else if name.starts_with("Box") {
                out.push(location(Provider::Box, "Box".into(), &dir));
            }
        }
    }

    // Windows desktop clients.
    let win_icloud = home.join("iCloudDrive");
    if win_icloud.is_dir() {
        out.push(location(Provider::ICloud, "iCloud Drive".into(), &win_icloud));
    }
    #[cfg(windows)]
    {
        let gdrive = PathBuf::from(r"G:\My Drive");
        if gdrive.is_dir() {
            out.push(location(Provider::GoogleDrive, "Google Drive".into(), &gdrive));
        }
        if let Some(onedrive) = std::env::var_os("OneDrive").map(PathBuf::from) {
            if onedrive.is_dir() && !out.iter().any(|l| l.provider == Provider::OneDrive) {
                out.push(location(Provider::OneDrive, "OneDrive".into(), &onedrive));
            }
        }
    }

    // Legacy/Linux Dropbox location.
    let legacy_dropbox = home.join("Dropbox");
    if legacy_dropbox.is_dir() && !out.iter().any(|l| l.provider == Provider::Dropbox) {
        out.push(location(Provider::Dropbox, "Dropbox".into(), &legacy_dropbox));
    }

    // Local-only: the home folder itself (never covered by iCloud's
    // "Desktop & Documents" sync, unlike ~/Documents).
    out.push(location(
        Provider::Local,
        "On this computer only".into(),
        home,
    ));
    out
}

/// Detect sync locations for the current user.
pub fn detect_locations() -> Vec<SyncLocation> {
    match dirs::home_dir() {
        Some(home) => detect_locations_in(&home),
        None => Vec::new(),
    }
}

/// Which provider (if any) syncs the folder at `path`.
pub fn provider_for_path(path: &Path) -> Provider {
    let p = path.to_string_lossy().replace('\\', "/");
    if p.contains("/Library/Mobile Documents/") || p.contains("/iCloudDrive") {
        Provider::ICloud
    } else if p.contains("/Library/CloudStorage/GoogleDrive-") || p.contains("/My Drive") {
        Provider::GoogleDrive
    } else if p.contains("/Library/CloudStorage/Dropbox") || p.contains("/Dropbox/") || p.ends_with("/Dropbox") {
        Provider::Dropbox
    } else if p.contains("/Library/CloudStorage/OneDrive") || p.contains("/OneDrive") {
        Provider::OneDrive
    } else if p.contains("/Library/CloudStorage/Box") {
        Provider::Box
    } else {
        Provider::Local
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn detects_cloud_folders_and_local_option() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        fs::create_dir_all(h.join("Library/Mobile Documents/com~apple~CloudDocs")).unwrap();
        fs::create_dir_all(h.join("Library/CloudStorage/GoogleDrive-me@gmail.com/My Drive")).unwrap();
        fs::create_dir_all(h.join("Library/CloudStorage/Dropbox")).unwrap();
        fs::create_dir_all(h.join("Library/CloudStorage/OneDrive-Personal")).unwrap();

        let found = detect_locations_in(h);
        let providers: Vec<Provider> = found.iter().map(|l| l.provider).collect();
        assert_eq!(
            providers,
            vec![
                Provider::ICloud,
                Provider::Dropbox,
                Provider::GoogleDrive,
                Provider::OneDrive,
                Provider::Local
            ]
        );
        let gdrive = found.iter().find(|l| l.provider == Provider::GoogleDrive).unwrap();
        assert!(gdrive.label.contains("me@gmail.com"));
        // Compare path components, so the test holds with Windows separators too.
        assert!(Path::new(&gdrive.vault_path).ends_with(Path::new("My Drive").join(VAULT_FOLDER_NAME)));
        assert!(!gdrive.vault_exists);
        let onedrive = found.iter().find(|l| l.provider == Provider::OneDrive).unwrap();
        assert_eq!(onedrive.label, "OneDrive (Personal)");
    }

    #[test]
    fn only_local_when_no_cloud_clients() {
        let home = tempfile::tempdir().unwrap();
        let found = detect_locations_in(home.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].provider, Provider::Local);
    }

    #[test]
    fn reports_existing_vault() {
        let home = tempfile::tempdir().unwrap();
        let icloud = home.path().join("Library/Mobile Documents/com~apple~CloudDocs");
        fs::create_dir_all(icloud.join("Satchel")).unwrap();
        let found = detect_locations_in(home.path());
        assert!(found[0].vault_exists);
    }

    #[test]
    fn provider_for_path_recognises_providers() {
        let cases = [
            ("/Users/a/Library/Mobile Documents/com~apple~CloudDocs/Satchel", Provider::ICloud),
            ("/Users/a/Library/CloudStorage/GoogleDrive-a@b.com/My Drive/Satchel", Provider::GoogleDrive),
            ("/Users/a/Library/CloudStorage/Dropbox/Satchel", Provider::Dropbox),
            ("/Users/a/Library/CloudStorage/OneDrive-Personal/Satchel", Provider::OneDrive),
            (r"C:\Users\a\iCloudDrive\Satchel", Provider::ICloud),
            ("/Users/a/Satchel", Provider::Local),
        ];
        for (path, want) in cases {
            assert_eq!(provider_for_path(Path::new(path)), want, "{path}");
        }
    }
}
