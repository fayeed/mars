//! Shared fixture for the permission-broker tests.
//!
//! This module is compiled into every test binary, so a helper used by one of
//! them looks unused to the others.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use mars_activity::ActivityRecord;
use mars_security::PermissionBroker;
use mars_storage::Storage;

pub struct BrokerFixture {
    // Declaration order is drop order: the database must close before the
    // directory holding it is removed.
    pub storage: Arc<Storage>,
    pub payload_root: PathBuf,
    pub worktree: PathBuf,
    _directory: TestDirectory,
}

impl BrokerFixture {
    pub fn new() -> Self {
        let directory = TestDirectory::new();
        let root = directory.path().join("repository");
        let worktree = directory.path().join("worktree");
        fs::create_dir_all(&root).expect("repository should exist");
        fs::create_dir_all(&worktree).expect("worktree should exist");
        let storage = Arc::new(
            Storage::open(directory.path().join("mars-test.sqlite3")).expect("storage should open"),
        );
        storage
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO projects (id, name, created_at, updated_at)
                     VALUES ('project-1', 'fixture', '2026-09-08T00:00:00Z',
                             '2026-09-08T00:00:00Z')",
                    [],
                )?;
                connection.execute(
                    "INSERT INTO repositories
                         (id, project_id, root_path, git_common_dir, state, created_at, updated_at)
                     VALUES ('repository-1', 'project-1', ?1, ?2, 'active',
                             '2026-09-08T00:00:00Z', '2026-09-08T00:00:00Z')",
                    rusqlite::params![
                        root.to_string_lossy(),
                        root.join(".git").to_string_lossy()
                    ],
                )?;
                Ok(())
            })
            .expect("fixture project and repository should insert");

        Self {
            storage,
            payload_root: directory.path().join("payloads"),
            worktree,
            _directory: directory,
        }
    }

    pub fn task(&self, id: &str) -> String {
        self.storage
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO tasks
                         (id, project_id, repository_id, title, intent, status, created_at,
                          updated_at)
                     VALUES (?1, 'project-1', 'repository-1', 'Fixture task', 'do the work',
                             'ready', '2026-09-08T00:00:00Z', '2026-09-08T00:00:00Z')",
                    [id],
                )?;
                Ok(())
            })
            .expect("task should insert");
        id.to_owned()
    }

    pub fn activity(&self) -> ActivityRecord {
        ActivityRecord::new(self.storage.clone(), &self.payload_root)
    }

    /// A broker that mirrors every request and decision into the activity log,
    /// which is how Mars actually runs it.
    pub fn broker(&self) -> PermissionBroker {
        PermissionBroker::new(self.storage.clone()).recording_to(self.activity())
    }

    /// Force a grant to look expired, so expiry can be tested without waiting.
    pub fn expire_grant(&self, grant_id: &str) {
        self.storage
            .with_connection(|connection| {
                connection.execute(
                    "UPDATE permission_grants SET expires_at = '2000-01-01T00:00:00.000Z'
                     WHERE id = ?1",
                    [grant_id],
                )?;
                Ok(())
            })
            .expect("grant should expire");
    }
}

impl Default for BrokerFixture {
    fn default() -> Self {
        Self::new()
    }
}

pub fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("test runtime should build")
}

pub struct TestDirectory(PathBuf);

impl TestDirectory {
    pub fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "mars-security-fixture-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("test directory should exist");
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if self.0.exists() {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
