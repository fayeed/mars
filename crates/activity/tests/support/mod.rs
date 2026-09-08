//! Shared fixture for the activity-record tests.
//!
//! This module is compiled into every test binary, so a helper used by one of
//! them looks unused to the others.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use mars_activity::{ActivityRecord, RunStore};
use mars_storage::Storage;

pub struct ActivityFixture {
    // Declaration order is drop order: the database must close before the
    // directory holding it is removed.
    pub storage: Arc<Storage>,
    pub repository_id: String,
    pub project_id: String,
    pub payload_root: PathBuf,
    pub worktree_root: PathBuf,
    _directory: TestDirectory,
}

impl ActivityFixture {
    pub fn new() -> Self {
        let directory = TestDirectory::new();
        let root = directory.path().join("repository");
        fs::create_dir_all(root.join(".git")).expect("repository should exist");
        fs::write(root.join("README.md"), "# fixture\n").expect("fixture source should write");
        let payload_root = directory.path().join("payloads");
        let worktree_root = directory.path().join("worktree");
        fs::create_dir_all(&worktree_root).expect("worktree root should exist");

        let storage = Arc::new(
            Storage::open(directory.path().join("mars-test.sqlite3")).expect("storage should open"),
        );
        // The projects and repositories rows exist only so the task foreign keys
        // resolve; nothing in this crate reads them.
        let project_id = "project-fixture".to_owned();
        let repository_id = "repository-fixture".to_owned();
        storage
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO projects (id, name, created_at, updated_at)
                     VALUES (?1, 'fixture', '2026-09-08T00:00:00Z', '2026-09-08T00:00:00Z')",
                    [&project_id],
                )?;
                connection.execute(
                    "INSERT INTO repositories
                         (id, project_id, root_path, git_common_dir, state, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, 'active',
                             '2026-09-08T00:00:00Z', '2026-09-08T00:00:00Z')",
                    rusqlite::params![
                        &repository_id,
                        &project_id,
                        root.to_string_lossy(),
                        root.join(".git").to_string_lossy(),
                    ],
                )?;
                Ok(())
            })
            .expect("fixture project and repository should insert");

        Self {
            storage,
            repository_id,
            project_id,
            payload_root,
            worktree_root,
            _directory: directory,
        }
    }

    pub fn record(&self) -> ActivityRecord {
        ActivityRecord::new(self.storage.clone(), &self.payload_root)
    }

    pub fn runs(&self) -> RunStore {
        RunStore::new(self.storage.clone())
    }

    /// Create a task row so activity has something to hang from.
    pub fn task(&self, id: &str, title: &str) -> String {
        self.storage
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO tasks
                         (id, project_id, repository_id, title, intent, status, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, 'do the work', 'ready',
                             '2026-09-08T00:00:00Z', '2026-09-08T00:00:00Z')",
                    rusqlite::params![id, &self.project_id, &self.repository_id, title],
                )?;
                Ok(())
            })
            .expect("task should insert");
        id.to_owned()
    }
}

impl Default for ActivityFixture {
    fn default() -> Self {
        Self::new()
    }
}

pub struct TestDirectory(PathBuf);

impl TestDirectory {
    pub fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "mars-activity-fixture-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("test directory should exist");
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Default for TestDirectory {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if self.0.exists() {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
