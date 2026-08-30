use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use mars_security::permissions::{
    Capability, PermissionAudit, PermissionDecision, PermissionOutcome, PermissionRequest,
    PermissionScope,
};
use mars_storage::Storage;

struct TestDatabase(PathBuf);

impl TestDatabase {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow epoch")
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "mars-permission-audit-{}-{nonce}.sqlite3",
            std::process::id()
        )))
    }
}

impl Drop for TestDatabase {
    fn drop(&mut self) {
        for suffix in ["", "-shm", "-wal"] {
            let path = PathBuf::from(format!("{}{}", self.0.display(), suffix));
            if path.exists() {
                fs::remove_file(path).expect("database artifact should remove");
            }
        }
    }
}

#[test]
fn decisions_are_durable_before_they_are_applied() {
    let database = TestDatabase::new();
    let storage = Arc::new(Storage::open(&database.0).expect("database should open"));
    let audit = PermissionAudit::new(storage.clone());
    let request = PermissionRequest::new(
        Capability::WriteFiles,
        PermissionScope::Worktree {
            task_id: "task-1".into(),
            root: PathBuf::from("/repo/worktree"),
        },
    );

    assert!(matches!(
        audit
            .record(&request, PermissionDecision::AllowSession)
            .expect("decision should record"),
        PermissionOutcome::Allowed(_)
    ));
    let stored: (String, String, String) = storage
        .with_connection(|connection| {
            connection.query_row(
                "SELECT capability, scope_json, decision FROM permission_decisions WHERE request_id = ?1",
                [&request.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
        })
        .expect("audit row should load");
    assert_eq!(stored.0, "write_files");
    assert!(stored.1.contains("task-1"));
    assert_eq!(stored.2, "allow_session");
}
