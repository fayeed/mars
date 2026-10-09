use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::time::{SystemTime, UNIX_EPOCH};

use mars_core::projects::{OpenRepositoryOutcome, ProjectError, ProjectService, canonicalize};
use mars_security::TrustDecision;
use mars_storage::Storage;

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "mars-open-repository-{name}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("test directory should be created");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if self.path.exists() {
            fs::remove_dir_all(&self.path).expect("test directory should be removable");
        }
    }
}

fn service_for(root: &TestDirectory) -> (Arc<Storage>, ProjectService) {
    let storage = Arc::new(
        Storage::open(root.path().join("mars-test.sqlite3")).expect("storage should open"),
    );
    let service = ProjectService::new(storage.clone());
    (storage, service)
}

#[test]
fn untrusted_repository_returns_a_canonical_candidate_without_persisting() {
    let fixture = TestDirectory::new("trust");
    let repository = fixture.path().join("repo");
    fs::create_dir_all(repository.join(".git")).expect("Git directory should exist");
    let (storage, service) = service_for(&fixture);

    let outcome = service
        .open_repository(repository.join("."), TrustDecision::RequireConfirmation)
        .expect("repository should be inspected");

    let OpenRepositoryOutcome::TrustRequired(candidate) = outcome else {
        panic!("untrusted repository should require confirmation");
    };
    assert_eq!(
        candidate.root_path,
        canonicalize(&repository).expect("root should resolve")
    );
    assert_eq!(
        candidate.git_common_dir,
        canonicalize(&repository.join(".git")).expect("Git directory should resolve")
    );
    assert_eq!(candidate.capabilities.len(), 3);
    let stored: i64 = storage
        .with_connection(|connection| {
            connection.query_row("SELECT COUNT(*) FROM repositories", [], |row| row.get(0))
        })
        .expect("repository count should load");
    assert_eq!(stored, 0);
}

#[test]
fn non_git_directories_are_rejected() {
    let fixture = TestDirectory::new("not-git");
    let repository = fixture.path().join("plain-directory");
    fs::create_dir(&repository).expect("plain directory should exist");
    let (_, service) = service_for(&fixture);

    let error = service
        .open_repository(&repository, TrustDecision::RequireConfirmation)
        .expect_err("non-Git directory should fail");

    assert!(matches!(error, ProjectError::NotGitRepository(path) if path == repository));
}

#[test]
fn git_file_worktrees_resolve_their_shared_common_directory() {
    let fixture = TestDirectory::new("worktree");
    let common = fixture.path().join("main/.git");
    let git_dir = common.join("worktrees/feature");
    let worktree = fixture.path().join("feature");
    fs::create_dir_all(&git_dir).expect("worktree metadata should exist");
    fs::create_dir_all(&worktree).expect("worktree should exist");
    fs::write(git_dir.join("commondir"), "../..\n").expect("commondir should be written");
    fs::write(
        worktree.join(".git"),
        "gitdir: ../main/.git/worktrees/feature\n",
    )
    .expect("Git file should be written");
    let (_, service) = service_for(&fixture);

    let outcome = service
        .open_repository(&worktree, TrustDecision::RequireConfirmation)
        .expect("worktree should be inspected");
    let OpenRepositoryOutcome::TrustRequired(candidate) = outcome else {
        panic!("worktree should require confirmation");
    };

    assert_eq!(
        candidate.git_common_dir,
        canonicalize(&common).expect("common dir should resolve")
    );
}

#[test]
fn malformed_git_files_are_rejected() {
    let fixture = TestDirectory::new("malformed-git-file");
    let repository = fixture.path().join("repo");
    fs::create_dir_all(&repository).expect("repository should exist");
    let git_file = repository.join(".git");
    fs::write(&git_file, "not a gitdir pointer\n").expect("Git file should be written");
    let (_, service) = service_for(&fixture);

    let error = service
        .open_repository(&repository, TrustDecision::RequireConfirmation)
        .expect_err("malformed Git file should fail");

    assert!(matches!(error, ProjectError::MalformedGitFile(path) if path == git_file));
}

#[test]
fn trusted_repository_open_is_idempotent() {
    let fixture = TestDirectory::new("idempotent");
    let repository = fixture.path().join("repo");
    fs::create_dir_all(repository.join(".git")).expect("Git directory should exist");
    let (storage, service) = service_for(&fixture);

    let first = service
        .open_repository(&repository, TrustDecision::Trust)
        .expect("repository should open");
    let second = service
        .open_repository(&repository, TrustDecision::Trust)
        .expect("repository should reopen");

    assert_eq!(first, second);
    let stored: i64 = storage
        .with_connection(|connection| {
            connection.query_row("SELECT COUNT(*) FROM repositories", [], |row| row.get(0))
        })
        .expect("repository count should load");
    assert_eq!(stored, 1);
}

#[test]
fn concurrent_trusted_opens_share_one_project() {
    const WORKERS: usize = 8;

    let fixture = TestDirectory::new("concurrent");
    let repository = fixture.path().join("repo");
    fs::create_dir_all(repository.join(".git")).expect("Git directory should exist");
    let (storage, service) = service_for(&fixture);
    let service = Arc::new(service);
    let barrier = Arc::new(Barrier::new(WORKERS));

    let handles: Vec<_> = (0..WORKERS)
        .map(|_| {
            let service = service.clone();
            let barrier = barrier.clone();
            let repository = repository.clone();
            std::thread::spawn(move || {
                barrier.wait();
                service
                    .open_repository(repository, TrustDecision::Trust)
                    .expect("repository should open")
            })
        })
        .collect();

    let outcomes: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().expect("worker should finish"))
        .collect();
    assert!(outcomes.iter().all(|outcome| outcome == &outcomes[0]));

    let stored: i64 = storage
        .with_connection(|connection| {
            connection.query_row("SELECT COUNT(*) FROM repositories", [], |row| row.get(0))
        })
        .expect("repository count should load");
    assert_eq!(stored, 1);
}
