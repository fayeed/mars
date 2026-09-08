#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use mars_core::projects::{OpenRepositoryOutcome, ProjectService};
use mars_security::TrustDecision;
use mars_storage::Storage;

pub struct GitFixture {
    // Declaration order is drop order: the database must close before the
    // directory holding it is removed, or Windows refuses to delete the file.
    pub storage: Arc<Storage>,
    pub repository_id: String,
    pub repository_root: PathBuf,
    pub worktrees_root: PathBuf,
    _directory: TestDirectory,
}

impl GitFixture {
    pub fn new() -> Self {
        let directory = TestDirectory::new();
        let repository_root = directory.path().join("repository with spaces");
        let worktrees_root = directory.path().join("Mars worktrees");
        fs::create_dir_all(&repository_root).expect("repository should exist");
        fs::create_dir_all(&worktrees_root).expect("worktree root should exist");
        git(
            &repository_root,
            &["init", "--quiet", "--initial-branch=main"],
        );
        fs::write(repository_root.join("README.md"), "# fixture\n")
            .expect("fixture source should write");
        git(&repository_root, &["add", "README.md"]);
        git(
            &repository_root,
            &[
                "-c",
                "user.name=Mars Test",
                "-c",
                "user.email=mars@example.test",
                "commit",
                "--quiet",
                "-m",
                "initial",
            ],
        );

        let storage = Arc::new(
            Storage::open(directory.path().join("mars.sqlite3")).expect("storage should open"),
        );
        let OpenRepositoryOutcome::Opened(project) = ProjectService::new(storage.clone())
            .open_repository(&repository_root, TrustDecision::Trust)
            .expect("repository should open")
        else {
            panic!("trusted repository should open")
        };

        Self {
            storage,
            repository_id: project.repository_id,
            repository_root,
            worktrees_root,
            _directory: directory,
        }
    }
}

impl Default for GitFixture {
    fn default() -> Self {
        Self::new()
    }
}

pub fn git(root: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .expect("system Git should launch");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow Unix epoch")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("mars-git-fixture-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).expect("test directory should exist");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        // Best effort: a failure to clean up must not turn an assertion failure
        // into a process abort by panicking while the first panic unwinds.
        if self.0.exists() {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
