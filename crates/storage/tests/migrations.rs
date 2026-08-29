use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use mars_storage::Storage;

struct TestDatabase {
    path: PathBuf,
}

impl TestDatabase {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow Unix epoch")
            .as_nanos();
        Self {
            path: std::env::temp_dir().join(format!(
                "mars-{name}-{}-{nonce}.sqlite3",
                std::process::id()
            )),
        }
    }

    fn path(&self) -> &Path {
        &self.path
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
fn fresh_database_applies_each_migration_once() {
    let database = TestDatabase::new("fresh-migration");
    let storage = Storage::open(database.path()).expect("fresh database should open");

    let applied: i64 = storage
        .with_connection(|connection| {
            connection.query_row("SELECT COUNT(*) FROM _mars_migrations", [], |row| {
                row.get(0)
            })
        })
        .expect("migration journal should be readable");
    let jobs_table: String = storage
        .with_connection(|connection| {
            connection.query_row(
                "SELECT name FROM sqlite_schema WHERE type = 'table' AND name = 'jobs'",
                [],
                |row| row.get(0),
            )
        })
        .expect("jobs table should exist");

    assert_eq!(applied, 3);
    assert_eq!(jobs_table, "jobs");

    drop(storage);
    let reopened = Storage::open(database.path()).expect("migrated database should reopen");
    let applied_after_reopen: i64 = reopened
        .with_connection(|connection| {
            connection.query_row("SELECT COUNT(*) FROM _mars_migrations", [], |row| {
                row.get(0)
            })
        })
        .expect("migration journal should remain readable");
    assert_eq!(applied_after_reopen, 3);
}

#[test]
fn connections_enable_wal_foreign_keys_and_fts5() {
    let database = TestDatabase::new("connection-settings");
    let storage = Storage::open(database.path()).expect("database should open");

    let (journal_mode, foreign_keys): (String, bool) = storage
        .with_connection(|connection| {
            let journal_mode = connection.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
            let foreign_keys = connection.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
            Ok((journal_mode, foreign_keys))
        })
        .expect("connection settings should be readable");

    assert_eq!(journal_mode.to_lowercase(), "wal");
    assert!(foreign_keys);
    storage
        .with_connection(|connection| {
            connection.execute_batch(
                "CREATE VIRTUAL TABLE fts5_probe USING fts5(body); DROP TABLE fts5_probe;",
            )
        })
        .expect("bundled SQLite should include FTS5");
}
