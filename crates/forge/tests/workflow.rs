use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use mars_forge::{
    CheckRun, CreatePullRequest, ForgeError, ForgeProvider, ForgeRepository, ForgeStore, ForgeUser,
    ForgeWorkflow, Issue, PullRequest, PullRequestPlan, ReviewComment,
};
use mars_storage::Storage;
use rusqlite::params;

struct Fixture {
    root: PathBuf,
    repository: PathBuf,
    bare: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "mars-forge-workflow-{}-{nonce}",
            std::process::id()
        ));
        let repository = root.join("repository");
        let bare = root.join("remote.git");
        fs::create_dir_all(&repository).expect("repository should create");
        git(&repository, &["init", "-b", "main"]);
        git(&repository, &["config", "user.name", "Mars Test"]);
        git(&repository, &["config", "user.email", "mars@example.test"]);
        fs::write(repository.join("sync.ts"), "export const mode = 'base';\n")
            .expect("base file should write");
        git(&repository, &["add", "sync.ts"]);
        git(&repository, &["commit", "-m", "base"]);
        git(
            &root,
            &["init", "--bare", bare.to_str().expect("bare path")],
        );
        git(
            &repository,
            &["remote", "add", "origin", "git@github.test:acme/mars.git"],
        );
        git(
            &repository,
            &[
                "remote",
                "set-url",
                "--push",
                "origin",
                bare.to_str().expect("bare path"),
            ],
        );
        git(&repository, &["checkout", "-b", "mars/fix-sync"]);
        fs::write(repository.join("sync.ts"), "export const mode = 'fixed';\n")
            .expect("implementation should write");
        git(&repository, &["add", "sync.ts"]);
        git(&repository, &["commit", "-m", "fix sync"]);
        Self {
            root,
            repository,
            bare,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if self.root.exists() {
            fs::remove_dir_all(&self.root).expect("fixture should remove");
        }
    }
}

struct FakeProvider {
    repository: ForgeRepository,
    head_sha: String,
    fail_create: bool,
    fail_comments: bool,
    /// Logins the workflow asked this provider to request a review from.
    requested: std::sync::Mutex<Vec<String>>,
    /// When true the forge refuses the request, as one does for a login that
    /// cannot review the repository.
    fail_reviewers: bool,
}

impl ForgeProvider for FakeProvider {
    fn repository(&self) -> &ForgeRepository {
        &self.repository
    }

    fn request_reviewers(
        &self,
        _number: u64,
        reviewers: &[String],
    ) -> Result<Vec<String>, ForgeError> {
        if self.fail_reviewers {
            return Err(ForgeError::Request("reviewer cannot be requested".into()));
        }
        self.requested
            .lock()
            .expect("requested reviewers lock")
            .extend_from_slice(reviewers);
        Ok(reviewers.to_vec())
    }

    fn current_user(&self) -> Result<ForgeUser, ForgeError> {
        Ok(ForgeUser {
            login: "faye".into(),
            url: "https://github.test/faye".into(),
        })
    }

    fn get_issue(&self, number: u64) -> Result<Issue, ForgeError> {
        Ok(Issue {
            provider_id: "issue-7".into(),
            number,
            title: "Fix sync".into(),
            body: "Preserve local state".into(),
            state: "open".into(),
            url: "https://github.test/acme/mars/issues/7".into(),
            author: "faye".into(),
            labels: vec!["bug".into()],
            updated_at: "2026-08-30T00:00:00Z".into(),
        })
    }

    fn create_pull_request(&self, input: &CreatePullRequest) -> Result<PullRequest, ForgeError> {
        if self.fail_create {
            return Err(ForgeError::Request(
                "ambiguous provider failure Authorization: Bearer ghp_supersecretvalue".into(),
            ));
        }
        Ok(PullRequest {
            provider_id: "pr-9".into(),
            number: 9,
            title: input.title.clone(),
            body: input.body.clone(),
            state: "open".into(),
            url: "https://github.test/acme/mars/pull/9".into(),
            head: input.head.clone(),
            base: input.base.clone(),
            draft: input.draft,
            mergeable: Some(true),
            head_sha: self.head_sha.clone(),
            updated_at: "2026-08-30T00:01:00Z".into(),
        })
    }

    fn get_pull_request(&self, number: u64) -> Result<PullRequest, ForgeError> {
        Ok(PullRequest {
            provider_id: "pr-9".into(),
            number,
            title: "Fix sync".into(),
            body: "Closes #7".into(),
            state: "open".into(),
            url: "https://github.test/acme/mars/pull/9".into(),
            head: "mars/fix-sync".into(),
            base: "main".into(),
            draft: false,
            mergeable: Some(true),
            head_sha: self.head_sha.clone(),
            updated_at: "2026-08-30T00:03:00Z".into(),
        })
    }

    fn list_review_comments(&self, _number: u64) -> Result<Vec<ReviewComment>, ForgeError> {
        if self.fail_comments {
            return Err(ForgeError::Request("comment refresh failed".into()));
        }
        Ok(vec![ReviewComment {
            provider_id: "comment-1".into(),
            author: "reviewer".into(),
            body: "Looks good".into(),
            path: None,
            line: None,
            url: "https://github.test/comment/1".into(),
            created_at: "2026-08-30T00:02:00Z".into(),
        }])
    }

    fn list_checks(&self, _revision: &str) -> Result<Vec<CheckRun>, ForgeError> {
        Ok(vec![CheckRun {
            provider_id: "check-1".into(),
            name: "test".into(),
            status: "completed".into(),
            conclusion: Some("success".into()),
            url: "https://github.test/check/1".into(),
            started_at: None,
            completed_at: None,
        }])
    }
}

#[test]
fn requires_approval_revalidates_and_pushes_exact_reviewed_plan() {
    let fixture = Fixture::new();
    let database = fixture.root.join("mars.sqlite3");
    let storage = Arc::new(Storage::open(database).expect("database should open"));
    seed(&storage, &fixture.repository);
    let workflow = ForgeWorkflow::new(storage.clone());
    let store = ForgeStore::new(storage);
    let issue_provider = FakeProvider {
        repository: repository(),
        head_sha: git_output(&fixture.repository, &["rev-parse", "HEAD"]),
        fail_create: false,
        fail_comments: false,
        requested: std::sync::Mutex::new(Vec::new()),
        fail_reviewers: false,
    };
    let issue = workflow
        .import_issue("repository", "origin", 7, &issue_provider)
        .expect("issue should import");
    workflow
        .link_issue_task(&issue.id, "task")
        .expect("issue should link");
    let plan = workflow
        .plan_pull_request("task", "origin", "Fix sync", "Closes #7", false, &[])
        .expect("PR should preview");
    assert_eq!(plan.commits_ahead, 1);
    assert!(plan.clean);
    assert!(plan.push_command.contains("HEAD:refs/heads/mars/fix-sync"));
    assert_eq!(plan.push_url, fixture.bare.to_string_lossy());
    assert!(matches!(
        workflow.apply_pull_request(&plan, false, false, &issue_provider),
        Err(ForgeError::ApprovalRequired)
    ));
    assert!(!remote_has_branch(&fixture.bare, "mars/fix-sync"));

    let failed = workflow.apply_pull_request(
        &plan,
        true,
        true,
        &FakeProvider {
            repository: repository(),
            head_sha: plan.head_sha.clone(),
            fail_create: true,
            fail_comments: false,
            requested: std::sync::Mutex::new(Vec::new()),
            fail_reviewers: false,
        },
    );
    let Err(ForgeError::Request(failed_message)) = failed else {
        panic!("provider failure should be preserved");
    };
    assert!(failed_message.contains("[REDACTED]"));
    assert!(!failed_message.contains("ghp_supersecretvalue"));
    assert!(remote_has_branch(&fixture.bare, "mars/fix-sync"));
    assert!(
        workflow
            .get_task_pull_request("task")
            .expect("stored PR lookup should work")
            .is_none(),
        "an ambiguous provider failure must not be reported as a persisted PR"
    );
    let failed_mutation = store
        .latest_mutation("task")
        .expect("mutation should load")
        .expect("failed mutation should exist");
    assert_eq!(failed_mutation.status, "failed");
    assert!(failed_mutation.push_approved);
    assert!(failed_mutation.forge_approved);
    assert!(
        failed_mutation
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("[REDACTED]")
    );
    assert!(
        !failed_mutation
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("ghp_supersecretvalue")
    );

    let partial = workflow.apply_pull_request(
        &plan,
        true,
        true,
        &FakeProvider {
            repository: repository(),
            head_sha: plan.head_sha.clone(),
            fail_create: false,
            fail_comments: true,
            requested: std::sync::Mutex::new(Vec::new()),
            fail_reviewers: false,
        },
    );
    assert!(matches!(partial, Err(ForgeError::Request(_))));
    let confirmed = workflow
        .get_task_pull_request("task")
        .expect("stored PR lookup should work")
        .expect("confirmed PR identity should survive refresh failure");
    assert_eq!(confirmed.pull_request.number, 9);
    assert!(confirmed.comments.is_empty());
    assert_eq!(
        store
            .latest_mutation("task")
            .expect("mutation should load")
            .expect("partial mutation should exist")
            .status,
        "failed"
    );

    let snapshot = workflow
        .apply_pull_request(&plan, true, true, &issue_provider)
        .expect("approved PR should publish");
    assert_eq!(snapshot.pull_request.number, 9);
    assert_eq!(snapshot.comments.len(), 1);
    assert_eq!(snapshot.checks.len(), 1);
    assert!(remote_has_branch(&fixture.bare, "mars/fix-sync"));
    let completed_mutation = store
        .latest_mutation("task")
        .expect("mutation should load")
        .expect("completed mutation should exist");
    assert_eq!(completed_mutation.status, "completed");
    assert!(completed_mutation.error.is_none());
    let refreshed = workflow
        .refresh_pull_request("task", &issue_provider)
        .expect("stored PR should refresh");
    assert_eq!(refreshed.pull_request.number, 9);
}

#[test]
fn rejects_credentials_embedded_in_an_http_push_url() {
    let fixture = Fixture::new();
    let database = fixture.root.join("mars.sqlite3");
    let storage = Arc::new(Storage::open(database).expect("database should open"));
    seed(&storage, &fixture.repository);
    git(
        &fixture.repository,
        &[
            "remote",
            "set-url",
            "--push",
            "origin",
            "https://ghp_supersecretvalue@github.test/acme/mars.git",
        ],
    );
    let workflow = ForgeWorkflow::new(storage);
    let error = workflow
        .plan_pull_request("task", "origin", "Fix sync", "Closes #7", false, &[])
        .expect_err("authenticated push URL should be rejected");
    assert!(matches!(error, ForgeError::UnsupportedRemote(_)));
    assert!(!error.to_string().contains("ghp_supersecretvalue"));
}

fn repository() -> ForgeRepository {
    ForgeRepository {
        host: "github.test".into(),
        owner: "acme".into(),
        name: "mars".into(),
    }
}

fn seed(storage: &Storage, repository: &Path) {
    storage
        .with_connection(|connection| {
            connection.execute(
                "INSERT INTO projects (id, name, created_at, updated_at) VALUES ('project', 'Mars', 't', 't')",
                [],
            )?;
            connection.execute(
                "INSERT INTO repositories
                 (id, project_id, root_path, git_common_dir, state, created_at, updated_at)
                 VALUES ('repository', 'project', ?1, ?2, 'active', 't', 't')",
                params![repository.to_string_lossy(), repository.join(".git").to_string_lossy()],
            )?;
            connection.execute(
                "INSERT INTO tasks
                 (id, project_id, repository_id, title, intent, status, created_at, updated_at)
                 VALUES ('task', 'project', 'repository', 'Fix sync', 'Preserve state', 'ready', 't', 't')",
                [],
            )?;
            connection.execute(
                "INSERT INTO branches
                 (id, repository_id, task_id, name, base_ref, head_oid, state, created_at, updated_at)
                 VALUES ('branch', 'repository', 'task', 'mars/fix-sync', 'main', NULL, 'created', 't', 't')",
                [],
            )?;
            connection.execute(
                "INSERT INTO worktrees
                 (id, repository_id, task_id, branch_id, path, state, created_at, updated_at)
                 VALUES ('worktree', 'repository', 'task', 'branch', ?1, 'ready', 't', 't')",
                [repository.to_string_lossy()],
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

fn git_output(root: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .expect("git should execute");
    assert!(output.status.success());
    String::from_utf8_lossy(&output.stdout).trim().into()
}

fn remote_has_branch(bare: &Path, branch: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(bare)
        .args(["show-ref", "--verify", &format!("refs/heads/{branch}")])
        .output()
        .expect("git should execute")
        .status
        .success()
}

#[test]
fn reviewers_are_part_of_the_approved_plan_and_requested_after_creation() {
    let fixture = Fixture::new();
    let database = fixture.root.join("mars.sqlite3");
    let storage = Arc::new(Storage::open(database).expect("database should open"));
    seed(&storage, &fixture.repository);
    let workflow = ForgeWorkflow::new(storage.clone());
    let head_sha = git_output(&fixture.repository, &["rev-parse", "HEAD"]);
    let provider = FakeProvider {
        repository: repository(),
        head_sha: head_sha.clone(),
        fail_create: false,
        fail_comments: false,
        requested: std::sync::Mutex::new(Vec::new()),
        fail_reviewers: false,
    };

    let reviewers = vec!["faye".to_owned(), "sam".to_owned()];
    let plan = workflow
        .plan_pull_request("task", "origin", "Fix sync", "Closes #7", false, &reviewers)
        .expect("plan should build");
    assert_eq!(plan.reviewers, reviewers);

    // Who gets asked to review is inside the fingerprint, so a plan approved
    // with one set of reviewers cannot be applied with another.
    let tampered = PullRequestPlan {
        reviewers: vec!["someone-else".to_owned()],
        ..plan.clone()
    };
    assert!(
        matches!(
            workflow.apply_pull_request(&tampered, true, true, &provider),
            Err(ForgeError::StalePlan)
        ),
        "changing the reviewers after approval must invalidate the plan"
    );

    workflow
        .apply_pull_request(&plan, true, true, &provider)
        .expect("approved plan should apply");
    assert_eq!(
        *provider.requested.lock().expect("requested lock"),
        reviewers,
        "the reviewers on the approved plan are the ones requested"
    );
}

#[test]
fn a_refused_reviewer_does_not_undo_the_pull_request_that_already_exists() {
    let fixture = Fixture::new();
    let database = fixture.root.join("mars.sqlite3");
    let storage = Arc::new(Storage::open(database).expect("database should open"));
    seed(&storage, &fixture.repository);
    let workflow = ForgeWorkflow::new(storage.clone());
    let provider = FakeProvider {
        repository: repository(),
        head_sha: git_output(&fixture.repository, &["rev-parse", "HEAD"]),
        fail_create: false,
        fail_comments: false,
        requested: std::sync::Mutex::new(Vec::new()),
        fail_reviewers: true,
    };

    let plan = workflow
        .plan_pull_request(
            "task",
            "origin",
            "Fix sync",
            "Closes #7",
            false,
            &["nope".into()],
        )
        .expect("plan should build");

    let snapshot = workflow
        .apply_pull_request(&plan, true, true, &provider)
        .expect("the pull request stands even when the reviewer request fails");
    assert_eq!(snapshot.pull_request.number, 9);
}
