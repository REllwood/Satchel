//! App-held state: the currently open vault and its connections.

use std::path::PathBuf;
use std::sync::Mutex;

use mynote_core::embed::Embedder;
use mynote_core::rusqlite::Connection;

/// An open vault: the writer connection, a sandboxed read-only query
/// connection, and a lazily-loaded embedder.
pub struct Vault {
    pub root: PathBuf,
    /// Single writer (the app), WAL mode.
    pub conn: Connection,
    /// `query_only` + authorizer connection for the in-app query view.
    pub query_conn: Connection,
    /// Loaded on first semantic use (model load is not free).
    pub embedder: Option<Embedder>,
    /// RAII guard: held only to keep the file watcher alive; dropped (stopping
    /// the watch) when the vault is replaced or closed.
    #[allow(dead_code)]
    pub watch: Option<mynote_core::watch::WatchGuard>,
}

/// Tauri-managed application state.
#[derive(Default)]
pub struct AppState {
    pub vault: Mutex<Option<Vault>>,
}
