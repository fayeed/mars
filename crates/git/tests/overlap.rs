use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use mars_git::{OverlapKind, TaskOverlapService, TaskWorktreeService};
use mars_storage::Storage;
use rusqlite::params;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow epoch")
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("mars-overlap-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&root).expect("fixture should create");
        git(&root, &["init", "-b", "main"]);
        git(&root, &["config", "user.name", "Mars Test"]);
        git(&root, &["config", "user.email", "mars@example.test"]);
        for (path, contents) in [
            ("a.ts", "export function alpha() { return 1; }\n"),
            ("b.ts", "export function beta() { return 2; }\n"),
            ("shared.ts", "export function shared() { return 3; }\n"),
        ] {
            fs::write(root.join(path), contents).expect("fixture source should write");
        }
        git(&root, &["add", "."]);
        git(&root, &["commit", "-m", "base"]);
        Self(root)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if self.0.exists() {
            fs::remove_dir_all(&self.0).expect("fixture should remove");
        }
    }
}

#[test]
fn compares_real_active_worktree_diffs_with_the_indexed_graph() {
    let fixture = Fixture::new();
    let storage =
        Arc::new(Storage::open(fixture.0.join("mars.sqlite3")).expect("database should open"));
    seed_repository_and_index(&storage, &fixture.0);
    let tasks = TaskWorktreeService::new(storage.clone(), fixture.0.join("worktrees"));
    let first = tasks
        .create_task("repository", "First", "Change alpha and shared", "main")
        .and_then(|task| tasks.provision(&task.task_id))
        .expect("first worktree should provision");
    let second = tasks
        .create_task("repository", "Second", "Change beta and shared", "main")
        .and_then(|task| tasks.provision(&task.task_id))
        .expect("second worktree should provision");
    fs::write(
        first.worktree_path.join("a.ts"),
        "export function alpha() { return 10; }\n",
    )
    .expect("first change should write");
    fs::write(
        first.worktree_path.join("shared.ts"),
        "export function shared() { return 30; }\n",
    )
    .expect("first shared change should write");
    fs::write(
        second.worktree_path.join("b.ts"),
        "export function beta() { return 20; }\n",
    )
    .expect("second change should write");
    fs::write(
        second.worktree_path.join("shared.ts"),
        "export function shared() { return 31; }\n",
    )
    .expect("second shared change should write");

    let warnings = TaskOverlapService::new(storage)
        .warnings(&first.task_id)
        .expect("overlap scan should succeed");
    for kind in [
        OverlapKind::ExactFile,
        OverlapKind::ChangedSymbol,
        OverlapKind::Dependency,
    ] {
        assert!(
            warnings.iter().any(|warning| warning.kind == kind),
            "missing {kind:?}: {warnings:#?}"
        );
    }
    assert!(
        warnings
            .iter()
            .all(|warning| warning.other_task_id == second.task_id)
    );
}

fn seed_repository_and_index(storage: &Storage, root: &Path) {
    storage
        .with_connection(|connection| {
            connection.execute(
                "INSERT INTO projects (id, name, created_at, updated_at) VALUES ('project', 'Mars', 't', 't')",
                [],
            )?;
            connection.execute(
                "INSERT INTO repositories
                 (id, project_id, root_path, git_common_dir, default_branch, state, created_at, updated_at)
                 VALUES ('repository', 'project', ?1, ?2, 'main', 'active', 't', 't')",
                params![root.to_string_lossy(), root.join(".git").to_string_lossy()],
            )?;
            connection.execute(
                "INSERT INTO index_runs
                 (id, repository_id, status, files_seen, started_at, finished_at)
                 VALUES ('run', 'repository', 'completed', 3, 't', 't')",
                [],
            )?;
            for (file_id, path, symbol_id, name) in [
                ("file-a", "a.ts", "symbol-a", "alpha"),
                ("file-b", "b.ts", "symbol-b", "beta"),
                ("file-shared", "shared.ts", "symbol-shared", "shared"),
            ] {
                connection.execute(
                    "INSERT INTO files
                     (id, repository_id, relative_path, language, content_hash, byte_size, parse_errors, indexed_at)
                     VALUES (?1, 'repository', ?2, 'typescript', 'hash', 40, 0, 't')",
                    params![file_id, path],
                )?;
                connection.execute(
                    "INSERT INTO symbols
                     (id, file_id, stable_key, kind, name, start_byte, end_byte, start_line, end_line, content_hash)
                     VALUES (?1, ?2, ?3, 'function', ?4, 0, 39, 1, 1, 'hash')",
                    params![symbol_id, file_id, format!("function:{name}"), name],
                )?;
            }
            connection.execute(
                "INSERT INTO code_edges
                 (id, repository_id, source_kind, source_id, edge_kind, target_kind, target_id,
                  target_key, confidence, evidence_json, index_run_id)
                 VALUES ('edge', 'repository', 'symbol', 'symbol-a', 'CALLS', 'symbol', 'symbol-b',
                         'function:beta', 0.9, '{}', 'run')",
                [],
            )?;
            Ok(())
        })
        .expect("fixtures should insert");
}

fn git(root: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .expect("git should execute");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        arguments,
        String::from_utf8_lossy(&output.stderr)
    );
}
