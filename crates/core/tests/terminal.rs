#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mars_core::projects::{OpenRepositoryOutcome, ProjectService};
use mars_core::terminal::{TerminalError, TerminalService};
use mars_security::TrustDecision;
use mars_storage::Storage;

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow Unix epoch")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("mars-terminal-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).expect("test directory should be created");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if self.0.exists() {
            fs::remove_dir_all(&self.0).expect("test directory should be removable");
        }
    }
}

fn terminal_fixture() -> (TestDirectory, TerminalService, String, PathBuf) {
    let fixture = TestDirectory::new();
    let repository = fixture.path().join("repo");
    fs::create_dir_all(repository.join(".git")).expect("Git directory should exist");
    let storage = Arc::new(
        Storage::open(fixture.path().join("mars-test.sqlite3")).expect("storage should open"),
    );
    let outcome = ProjectService::new(storage.clone())
        .open_repository(&repository, TrustDecision::Trust)
        .expect("repository should open");
    let OpenRepositoryOutcome::Opened(project) = outcome else {
        panic!("trusted repository should open")
    };
    (
        fixture,
        TerminalService::new(storage),
        project.repository_id,
        repository.canonicalize().expect("root should resolve"),
    )
}

#[test]
fn terminal_starts_in_repository_and_accepts_resize() {
    let (_fixture, service, repository_id, root) = terminal_fixture();
    let session = service
        .spawn_terminal(&repository_id, 80, 24, Arc::new(|_| {}))
        .expect("terminal should start");

    assert_eq!(session.cwd, root);
    service
        .resize_terminal(&session.session_id, 120, 40)
        .expect("terminal should resize");
    service
        .write_terminal(&session.session_id, "printf '__MARS_CWD__%s\\n' \"$PWD\"\n")
        .expect("terminal should accept input");

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let output = service
            .terminal_output(&session.session_id)
            .expect("terminal output should load");
        if output.contains("__MARS_CWD__") && output.contains(&root.display().to_string()) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "terminal did not report its working directory"
        );
        thread::sleep(Duration::from_millis(20));
    }

    service
        .cancel_terminal(&session.session_id)
        .expect("terminal should cancel");
}

#[test]
fn cancellation_closes_terminal_input() {
    let (_fixture, service, repository_id, _) = terminal_fixture();
    let session = service
        .spawn_terminal(&repository_id, 80, 24, Arc::new(|_| {}))
        .expect("terminal should start");

    service
        .cancel_terminal(&session.session_id)
        .expect("terminal should cancel");

    assert!(matches!(
        service.write_terminal(&session.session_id, "echo late\n"),
        Err(TerminalError::SessionNotFound(_))
    ));
}
