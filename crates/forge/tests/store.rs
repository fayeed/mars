use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use mars_forge::{CheckRun, ForgeRepository, ForgeStore, Issue, PullRequest, ReviewComment};
use mars_storage::Storage;
use rusqlite::params;

struct TestDatabase(PathBuf);

impl TestDatabase {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow epoch")
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "mars-forge-store-{}-{nonce}.sqlite3",
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
fn persists_issue_task_and_pr_snapshot_idempotently() {
    let database = TestDatabase::new();
    let storage = Arc::new(Storage::open(&database.0).expect("database should open"));
    storage
        .with_connection(|connection| {
            connection.execute(
                "INSERT INTO projects (id, name, created_at, updated_at) VALUES ('p', 'Mars', 't', 't')",
                [],
            )?;
            connection.execute(
                "INSERT INTO repositories
                 (id, project_id, root_path, git_common_dir, state, created_at, updated_at)
                 VALUES ('r', 'p', '/repo', '/repo/.git', 'active', 't', 't')",
                [],
            )?;
            connection.execute(
                "INSERT INTO tasks
                 (id, project_id, repository_id, title, intent, status, created_at, updated_at)
                 VALUES ('task', 'p', 'r', 'Fix sync', 'Keep state', 'ready', 't', 't')",
                [],
            )?;
            Ok(())
        })
        .expect("fixtures should insert");
    let store = ForgeStore::new(storage.clone());
    let repository = ForgeRepository {
        host: "github.com".into(),
        owner: "acme".into(),
        name: "mars".into(),
    };
    let remote = store
        .upsert_remote(
            "r",
            "origin",
            "git@github.com:acme/mars.git",
            "github",
            &repository,
        )
        .expect("remote should persist");
    let connection = store
        .upsert_connection(
            &remote.id,
            "github",
            "https://api.github.com",
            Some("faye"),
            "github:remote:faye",
        )
        .expect("connection metadata should persist");
    assert_eq!(
        store
            .get_connection(&remote.id)
            .expect("connection should load"),
        Some(connection)
    );
    let stored_secret: bool = storage
        .with_connection(|connection| {
            connection.query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM forge_connections
                   WHERE credential_ref = 'secret-token' OR account_login = 'secret-token'
                      OR api_base = 'secret-token' OR provider = 'secret-token'
                 )",
                [],
                |row| row.get(0),
            )
        })
        .expect("credential metadata should query");
    assert!(!stored_secret, "raw token must never enter SQLite");
    let issue = sample_issue();
    let stored_issue = store
        .upsert_issue(&remote.id, &issue)
        .expect("issue should persist");
    assert_eq!(
        store
            .upsert_issue(&remote.id, &issue)
            .expect("issue should update")
            .id,
        stored_issue.id
    );
    store
        .link_issue_task(&stored_issue.id, "task")
        .expect("issue should link");

    let snapshot = store
        .record_pull_request(
            &remote.id,
            Some("task"),
            Some(&stored_issue.id),
            &sample_pull_request(),
            &[sample_comment()],
            &[sample_check()],
        )
        .expect("PR snapshot should persist");
    assert_eq!(snapshot.comments.len(), 1);
    assert_eq!(snapshot.checks.len(), 1);
    assert_eq!(
        store
            .get_task_pull_request("task")
            .expect("stored PR should load")
            .expect("stored PR should exist")
            .pull_request
            .number,
        9
    );
    let counts: (i64, i64, i64, i64) = storage
        .with_connection(|connection| {
            connection.query_row(
                "SELECT
                   (SELECT COUNT(*) FROM issues),
                   (SELECT COUNT(*) FROM pull_requests),
                   (SELECT COUNT(*) FROM review_comments),
                   (SELECT COUNT(*) FROM check_runs)",
                params![],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
        })
        .expect("counts should query");
    assert_eq!(counts, (1, 1, 1, 1));

    let mutation_id = store
        .start_mutation("task", &remote.id, "fingerprint", true, true, "{}")
        .expect("mutation should start");
    assert_eq!(
        store
            .recover_interrupted_mutations()
            .expect("mutation should recover"),
        1
    );
    let mutation = store
        .latest_mutation("task")
        .expect("mutation should load")
        .expect("mutation should exist");
    assert_eq!(mutation.id, mutation_id);
    assert_eq!(mutation.status, "interrupted");
    assert!(mutation.finished_at.is_some());
}

fn sample_issue() -> Issue {
    Issue {
        provider_id: "101".into(),
        number: 7,
        title: "Fix sync".into(),
        body: "Keep local state".into(),
        state: "open".into(),
        url: "https://github.com/acme/mars/issues/7".into(),
        author: "faye".into(),
        labels: vec!["bug".into()],
        updated_at: "2026-08-30T00:00:00Z".into(),
    }
}

fn sample_pull_request() -> PullRequest {
    PullRequest {
        provider_id: "201".into(),
        number: 9,
        title: "Fix sync".into(),
        body: "Closes #7".into(),
        state: "open".into(),
        url: "https://github.com/acme/mars/pull/9".into(),
        head: "mars/fix-sync".into(),
        base: "main".into(),
        draft: false,
        mergeable: Some(true),
        head_sha: "abc123".into(),
        updated_at: "2026-08-30T00:01:00Z".into(),
    }
}

fn sample_comment() -> ReviewComment {
    ReviewComment {
        provider_id: "301".into(),
        author: "reviewer".into(),
        body: "Guard this branch".into(),
        path: Some("src/sync.ts".into()),
        line: Some(12),
        url: "https://github.com/comment/301".into(),
        created_at: "2026-08-30T00:02:00Z".into(),
    }
}

fn sample_check() -> CheckRun {
    CheckRun {
        provider_id: "401".into(),
        name: "test".into(),
        status: "completed".into(),
        conclusion: Some("success".into()),
        url: "https://github.com/check/401".into(),
        started_at: Some("2026-08-30T00:03:00Z".into()),
        completed_at: Some("2026-08-30T00:04:00Z".into()),
    }
}
