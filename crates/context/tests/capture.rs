use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use mars_context::{CaptureError, ContextKind, ContextService};
use mars_core::projects::{OpenRepositoryOutcome, ProjectService};
use mars_git::TaskWorktreeService;
use mars_security::TrustDecision;
use mars_storage::Storage;

#[test]
fn text_capture_deduplicates_attaches_and_searches_by_task() {
    let fixture = Fixture::new();
    let first = fixture
        .context
        .capture_text(
            &fixture.repository_id,
            Some(&fixture.task_id),
            Some("Parser notes"),
            "Malformed records must remain recoverable.",
        )
        .expect("text should capture");
    let duplicate = fixture
        .context
        .capture_text(
            &fixture.repository_id,
            Some(&fixture.second_task_id),
            Some("Duplicate title does not replace original"),
            "Malformed records must remain recoverable.",
        )
        .expect("duplicate text should reuse its item");

    assert_eq!(first.id, duplicate.id);
    assert_eq!(first.kind, ContextKind::Text);
    assert_eq!(duplicate.title, "Parser notes");
    assert_eq!(
        fixture
            .context
            .search(
                &fixture.repository_id,
                Some(&fixture.task_id),
                "recoverable",
                10,
            )
            .expect("task context should search"),
        vec![first.clone()],
    );
    assert_eq!(
        fixture
            .context
            .search(
                &fixture.repository_id,
                Some(&fixture.second_task_id),
                "Parser",
                10,
            )
            .expect("deduplicated context should attach to another task"),
        vec![first],
    );
}

#[test]
fn image_capture_preserves_an_exact_verified_original() {
    let fixture = Fixture::new();
    let source = fixture.directory.path().join("settings mock.png");
    let original = b"\x89PNG\r\n\x1a\nMARS_IMAGE_FIXTURE";
    fs::write(&source, original).expect("image fixture should write");

    let first = fixture
        .context
        .capture_file(
            &fixture.repository_id,
            Some(&fixture.task_id),
            Some("Settings mock"),
            &source,
            "image/png",
        )
        .expect("image should capture");
    let duplicate = fixture
        .context
        .capture_file(
            &fixture.repository_id,
            Some(&fixture.task_id),
            None,
            &source,
            "image/png",
        )
        .expect("same image should deduplicate");
    assert_eq!(first.id, duplicate.id);
    assert_eq!(first.kind, ContextKind::Image);

    fs::write(&source, b"source changed after capture").expect("source should change");
    let artifact_path = first.artifact_path.expect("image should have an artifact");
    assert_eq!(
        fs::read(artifact_path).expect("artifact should reopen"),
        original
    );
}

#[test]
fn failed_attachment_transaction_removes_a_new_artifact() {
    let fixture = Fixture::new();
    let source = fixture.directory.path().join("orphan.txt");
    fs::write(&source, "transaction should fail").expect("fixture should write");

    assert!(matches!(
        fixture.context.capture_file(
            &fixture.repository_id,
            Some("missing-task"),
            None,
            &source,
            "text/plain",
        ),
        Err(CaptureError::Storage(_))
    ));
    assert!(artifact_files(&fixture.artifact_root).is_empty());
    let count: i64 = fixture
        .storage
        .with_connection(|connection| {
            connection.query_row("SELECT COUNT(*) FROM context_items", [], |row| row.get(0))
        })
        .expect("context rows should count");
    assert_eq!(count, 0);
}

#[test]
fn detected_secrets_are_quarantined_and_never_attached() {
    let fixture = Fixture::new();
    let item = fixture
        .context
        .capture_text(
            &fixture.repository_id,
            Some(&fixture.task_id),
            Some("Do not send"),
            "API_KEY=super-secret-value",
        )
        .expect("secret should be preserved in quarantine");

    assert_eq!(
        item.quarantine_reason.as_deref(),
        Some("credential assignment pattern detected"),
    );
    assert!(matches!(
        fixture.context.attach_to_task(&item.id, &fixture.task_id),
        Err(CaptureError::Quarantined(_))
    ));
    assert!(
        fixture
            .context
            .search(&fixture.repository_id, Some(&fixture.task_id), "secret", 10,)
            .expect("task search should work")
            .is_empty()
    );
}

fn artifact_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if !root.exists() {
        return files;
    }
    let mut directories = vec![root.to_owned()];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(directory).expect("artifact directory should read") {
            let path = entry.expect("artifact entry should read").path();
            if path.is_dir() {
                directories.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files
}

struct Fixture {
    directory: TestDirectory,
    repository_id: String,
    task_id: String,
    second_task_id: String,
    storage: Arc<Storage>,
    artifact_root: PathBuf,
    context: ContextService,
}

impl Fixture {
    fn new() -> Self {
        let directory = TestDirectory::new();
        let repository_root = directory.path().join("repository");
        let worktrees_root = directory.path().join("worktrees");
        let artifact_root = directory.path().join("artifacts");
        fs::create_dir_all(&repository_root).expect("repository should exist");
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
        let tasks = TaskWorktreeService::new(storage.clone(), &worktrees_root);
        let task_id = tasks
            .create_task(&project.repository_id, "First task", "first", "HEAD")
            .expect("first task should plan")
            .task_id;
        let second_task_id = tasks
            .create_task(&project.repository_id, "Second task", "second", "HEAD")
            .expect("second task should plan")
            .task_id;
        let context = ContextService::new(storage.clone(), &artifact_root);
        Self {
            directory,
            repository_id: project.repository_id,
            task_id,
            second_task_id,
            storage,
            artifact_root,
            context,
        }
    }
}

fn git(root: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .expect("Git should launch");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "mars-context-fixture-{}-{nonce}",
            std::process::id(),
        ));
        fs::create_dir_all(&path).expect("fixture directory should exist");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if self.0.exists() {
            fs::remove_dir_all(&self.0).expect("fixture should be removable");
        }
    }
}
