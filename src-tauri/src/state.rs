//! App-held state: the currently open vault and its connections.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use satchel_core::embed::Embedder;
use satchel_core::rusqlite::Connection;

/// An open vault: the writer connection, a sandboxed read-only query
/// connection, and a lazily-loaded embedder.
pub struct Vault {
    pub root: PathBuf,
    /// Single writer (the app), WAL mode.
    pub conn: Connection,
    /// `query_only` + authorizer connection for the in-app query view.
    pub query_conn: Connection,
    /// Loaded by the background semantic indexer (model load is not free).
    /// Shared so the indexer can run the model without holding the vault lock.
    pub embedder: Option<Arc<Embedder>>,
    /// RAII guard: held only to keep the file watcher alive; dropped (stopping
    /// the watch) when the vault is replaced or closed.
    #[allow(dead_code)]
    pub watch: Option<satchel_core::watch::WatchGuard>,
}

/// Coalesces requests for the background semantic indexer. At most one worker
/// runs, and a request made while it runs sends it round for another pass, so
/// a note changed mid-pass is never left without embeddings.
#[derive(Default)]
pub struct EmbedQueue {
    running: AtomicBool,
    again: AtomicBool,
}

impl EmbedQueue {
    /// Ask for a pass. Returns true when the caller must start the worker.
    pub fn request(&self) -> bool {
        self.again.store(true, Ordering::SeqCst);
        !self.running.swap(true, Ordering::SeqCst)
    }

    /// The worker is starting a pass, which covers every request made so far.
    pub fn begin_pass(&self) {
        self.again.store(false, Ordering::SeqCst);
    }

    /// The worker finished a pass. Returns true when it must run another, or
    /// false when it has stopped.
    pub fn end_pass(&self) -> bool {
        if self.again.load(Ordering::SeqCst) {
            return true;
        }
        self.release()
    }

    /// Mark the worker stopped. A request made between `end_pass`'s check and
    /// this store saw the worker still running and left the work to it, so
    /// take the work back unless a new worker has already started.
    fn release(&self) -> bool {
        self.running.store(false, Ordering::SeqCst);
        self.again.load(Ordering::SeqCst) && !self.running.swap(true, Ordering::SeqCst)
    }

    /// The worker gave up after an error. The next request starts a new one.
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

/// Tauri-managed application state.
pub struct AppState {
    pub vault: Mutex<Option<Vault>>,
    /// The bundled embedding model, resolved from the app's resources.
    pub model_dir: PathBuf,
    /// The background semantic indexer's run state.
    pub embed_queue: EmbedQueue,
}

impl AppState {
    pub fn new(model_dir: PathBuf) -> Self {
        Self {
            vault: Mutex::new(None),
            model_dir,
            embed_queue: EmbedQueue::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::thread;

    #[test]
    fn only_the_first_request_starts_a_worker() {
        let q = EmbedQueue::default();
        assert!(q.request());
        assert!(!q.request());
        q.begin_pass();
        assert!(!q.end_pass());
        assert!(!q.is_running());
        assert!(q.request(), "an idle queue starts a new worker");
    }

    #[test]
    fn a_request_during_a_pass_runs_another_pass() {
        let q = EmbedQueue::default();
        assert!(q.request());
        q.begin_pass();
        assert!(!q.request());
        assert!(q.end_pass());
        q.begin_pass();
        assert!(!q.end_pass());
    }

    #[test]
    fn a_request_made_while_the_worker_stops_is_kept() {
        let q = EmbedQueue::default();
        assert!(q.request());
        q.begin_pass();
        // The worker has checked for new requests and found none...
        assert!(!q.request(), "the worker still looks busy");
        // ...so stopping must notice this request and keep going.
        assert!(q.release());
        assert!(q.is_running());
    }

    #[test]
    fn stop_lets_the_next_request_start_a_worker() {
        let q = EmbedQueue::default();
        assert!(q.request());
        q.begin_pass();
        q.stop();
        assert!(q.request());
    }

    /// Under contention, passes never overlap, and the last request is always
    /// followed by a pass that began after it.
    #[test]
    fn passes_never_overlap_and_the_last_request_is_served() {
        for _ in 0..50 {
            let q = Arc::new(EmbedQueue::default());
            let made = Arc::new(AtomicUsize::new(0));
            let seen = Arc::new(AtomicUsize::new(0));
            let in_pass = Arc::new(AtomicUsize::new(0));
            let workers = Arc::new(Mutex::new(Vec::new()));

            let requesters: Vec<_> = (0..4)
                .map(|_| {
                    let (q, made, seen, in_pass, workers) = (
                        q.clone(),
                        made.clone(),
                        seen.clone(),
                        in_pass.clone(),
                        workers.clone(),
                    );
                    thread::spawn(move || {
                        for _ in 0..200 {
                            made.fetch_add(1, Ordering::SeqCst);
                            if !q.request() {
                                continue;
                            }
                            let (q, made, seen, in_pass) =
                                (q.clone(), made.clone(), seen.clone(), in_pass.clone());
                            workers.lock().unwrap().push(thread::spawn(move || loop {
                                q.begin_pass();
                                assert_eq!(in_pass.fetch_add(1, Ordering::SeqCst), 0);
                                seen.fetch_max(made.load(Ordering::SeqCst), Ordering::SeqCst);
                                in_pass.fetch_sub(1, Ordering::SeqCst);
                                if !q.end_pass() {
                                    break;
                                }
                            }));
                        }
                    })
                })
                .collect();
            for r in requesters {
                r.join().unwrap();
            }
            for w in std::mem::take(&mut *workers.lock().unwrap()) {
                w.join().unwrap();
            }
            assert!(!q.is_running());
            assert_eq!(seen.load(Ordering::SeqCst), made.load(Ordering::SeqCst));
        }
    }
}
