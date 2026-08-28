use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use mars_core::jobs::{JobRegistry, JobStatus};
use mars_storage::Storage;
use serde_json::json;

struct TestDatabase {
    path: PathBuf,
}

impl TestDatabase {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow Unix epoch")
            .as_nanos();
        Self {
            path: std::env::temp_dir().join(format!(
                "mars-job-recovery-{}-{nonce}.sqlite3",
                std::process::id()
            )),
        }
    }
}

impl Drop for TestDatabase {
    fn drop(&mut self) {
        for suffix in ["", "-shm", "-wal"] {
            let candidate = PathBuf::from(format!("{}{}", self.path.display(), suffix));
            if candidate.exists() {
                fs::remove_file(candidate).expect("test database artifact should be removable");
            }
        }
    }
}

#[test]
fn events_are_persisted_in_monotonic_order() {
    let database = TestDatabase::new();
    let storage = Arc::new(Storage::open(&database.path).expect("storage should open"));
    let registry = JobRegistry::new(storage);
    registry
        .create("job-1", "index")
        .expect("job should be created");
    registry.start("job-1").expect("job should start");

    registry
        .append_event("job-1", "index.started", json!({ "root": "/repo" }))
        .expect("first event should persist");
    registry
        .append_event("job-1", "index.progress", json!({ "filesSeen": 3 }))
        .expect("second event should persist");

    let events = registry.events("job-1").expect("events should load");
    assert_eq!(
        events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(events[1].payload, json!({ "filesSeen": 3 }));
}

#[test]
fn cancellation_updates_durable_state_and_signals_the_live_token() {
    let database = TestDatabase::new();
    let storage = Arc::new(Storage::open(&database.path).expect("storage should open"));
    let registry = JobRegistry::new(storage);
    registry
        .create("job-1", "index")
        .expect("job should be created");
    let token = registry.start("job-1").expect("job should start");

    registry.cancel("job-1").expect("job should cancel");

    assert!(token.is_cancelled());
    assert_eq!(
        registry.status("job-1").expect("job should load"),
        JobStatus::CancelRequested
    );
}

#[test]
fn reopening_marks_abandoned_running_jobs_interrupted() {
    let database = TestDatabase::new();
    {
        let storage = Arc::new(Storage::open(&database.path).expect("storage should open"));
        let registry = JobRegistry::new(storage);
        registry
            .create("job-1", "index")
            .expect("job should be created");
        registry.start("job-1").expect("job should start");
    }

    let reopened = Arc::new(Storage::open(&database.path).expect("storage should reopen"));
    let recovered_registry = JobRegistry::new(reopened);
    let recovered = recovered_registry
        .recover_interrupted()
        .expect("recovery should succeed");

    assert_eq!(recovered, ["job-1"]);
    assert_eq!(
        recovered_registry.status("job-1").expect("job should load"),
        JobStatus::Interrupted
    );
}

#[test]
fn terminal_jobs_reject_further_state_changes() {
    let database = TestDatabase::new();
    let storage = Arc::new(Storage::open(&database.path).expect("storage should open"));
    let registry = JobRegistry::new(storage);
    registry
        .create("job-1", "index")
        .expect("job should be created");
    registry.start("job-1").expect("job should start");
    assert!(registry.start("job-1").is_err());

    registry
        .complete("job-1")
        .expect("running job should complete");

    assert_eq!(
        registry.status("job-1").expect("job should load"),
        JobStatus::Completed
    );
    assert!(registry.cancel("job-1").is_err());
    assert!(registry.fail("job-1").is_err());
}
