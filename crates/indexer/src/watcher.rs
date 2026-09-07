//! Working-tree watching with per-path debouncing.
//!
//! Filesystem events arrive in bursts: an editor save can produce several
//! events for one file, and a branch switch rewrites hundreds. Each path is
//! debounced independently and flushed once quiet, so a burst costs one
//! targeted re-index rather than one per event.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use mars_storage::Storage;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::{IndexError, IndexSummary, Indexer, walk};

/// How long a path must stay quiet before it is re-indexed.
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(250);

/// How often the debounce loop checks for paths that have gone quiet.
const TICK: Duration = Duration::from_millis(25);

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "event")]
pub enum WatchEvent {
    /// Paths went quiet and are about to be re-indexed.
    Changed {
        relative_paths: Vec<String>,
    },
    /// The repository moved to a different commit, so the whole tree is re-read.
    HeadChanged,
    Indexed {
        summary: IndexSummary,
    },
    Failed {
        error: String,
    },
}

#[derive(Clone, Debug)]
pub struct WatchOptions {
    pub debounce: Duration,
}

impl Default for WatchOptions {
    fn default() -> Self {
        Self {
            debounce: DEFAULT_DEBOUNCE,
        }
    }
}

/// A running watch. Dropping it stops the watcher and joins its thread.
pub struct RepositoryWatcher {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    _watcher: RecommendedWatcher,
}

impl RepositoryWatcher {
    /// Starts watching `root`, re-indexing debounced changes into the index.
    pub fn start(
        storage: Arc<Storage>,
        repository_id: &str,
        root: &Path,
        options: WatchOptions,
        observer: impl FnMut(&WatchEvent) + Send + 'static,
    ) -> Result<Self, IndexError> {
        let (sender, receiver): (Sender<Event>, Receiver<Event>) = channel();
        let mut watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
            if let Ok(event) = result {
                // A closed receiver just means the watch is shutting down.
                let _ = sender.send(event);
            }
        })
        .map_err(|error| IndexError::Watch(error.to_string()))?;
        watcher
            .watch(root, RecursiveMode::Recursive)
            .map_err(|error| IndexError::Watch(error.to_string()))?;

        let stop = Arc::new(AtomicBool::new(false));
        let worker = std::thread::spawn({
            let stop = stop.clone();
            let repository_id = repository_id.to_owned();
            let root = root.to_path_buf();
            let observer = Mutex::new(observer);
            move || {
                debounce_loop(
                    &storage,
                    &repository_id,
                    &root,
                    &options,
                    &receiver,
                    &stop,
                    &observer,
                );
            }
        });

        Ok(Self {
            stop,
            worker: Some(worker),
            _watcher: watcher,
        })
    }

    /// Stops the watch and waits for the worker to finish.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for RepositoryWatcher {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[allow(clippy::too_many_arguments)]
fn debounce_loop(
    storage: &Arc<Storage>,
    repository_id: &str,
    root: &Path,
    options: &WatchOptions,
    receiver: &Receiver<Event>,
    stop: &Arc<AtomicBool>,
    observer: &Mutex<impl FnMut(&WatchEvent) + Send>,
) {
    let indexer = Indexer::new(storage.clone());
    let mut pending: HashMap<String, Instant> = HashMap::new();
    let mut head_changed = false;

    while !stop.load(Ordering::SeqCst) {
        match receiver.recv_timeout(TICK) {
            Ok(event) => {
                for path in event.paths {
                    match classify(root, &path) {
                        Some(Change::Head) => head_changed = true,
                        Some(Change::Path(relative)) => {
                            pending.insert(relative, Instant::now());
                        }
                        None => {}
                    }
                }
                continue;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        let ready: Vec<String> = pending
            .iter()
            .filter(|(_, seen)| seen.elapsed() >= options.debounce)
            .map(|(path, _)| path.clone())
            .collect();
        if !head_changed && ready.is_empty() {
            continue;
        }
        for path in &ready {
            pending.remove(path);
        }

        let notify = |event: &WatchEvent| {
            if let Ok(mut observer) = observer.lock() {
                observer(event);
            }
        };

        // A commit change rewrites arbitrarily many files, so the whole tree is
        // re-read rather than trusting the event stream to have reported each one.
        let outcome = if head_changed {
            head_changed = false;
            pending.clear();
            notify(&WatchEvent::HeadChanged);
            indexer.index_repository(repository_id, &CancellationToken::new())
        } else {
            let mut paths = ready;
            paths.sort();
            notify(&WatchEvent::Changed {
                relative_paths: paths.clone(),
            });
            indexer.reindex_paths(repository_id, paths, &CancellationToken::new())
        };

        match outcome {
            Ok(summary) => notify(&WatchEvent::Indexed { summary }),
            Err(error) => notify(&WatchEvent::Failed {
                error: error.to_string(),
            }),
        }
    }
}

enum Change {
    Head,
    Path(String),
}

/// Maps a filesystem event path to something the index cares about. Git's own
/// bookkeeping is ignored apart from `HEAD`, which signals a branch switch.
fn classify(root: &Path, path: &Path) -> Option<Change> {
    let relative = walk::relative_path(root, path).ok()?;
    if relative == ".git/HEAD" {
        return Some(Change::Head);
    }
    if relative.starts_with(".git/") || relative == ".git" {
        return None;
    }
    if relative.is_empty() {
        return None;
    }
    Some(Change::Path(relative))
}
