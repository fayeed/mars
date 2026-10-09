#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use mars_agents::adapter::{
    AgentAdapter, AgentLaunchRequest, JsonLineDecoder, NormalizedAgentEvent,
};
use mars_agents::codex::CodexAdapter;
use mars_agents::process::{ProcessEventKind, ProcessSpec, ProcessSupervisor, SessionStatus};
use mars_core::projects::{OpenRepositoryOutcome, ProjectService};
use mars_git::{TaskWorktreeService, WorktreeState};
use mars_security::TrustDecision;
use mars_storage::Storage;

#[test]
#[ignore = "requires authenticated installed Codex CLI and explicitly permits a paid agent run"]
fn task_worktree_real_agent_stream_and_reopen() {
    tokio::runtime::Runtime::new()
        .expect("Tokio runtime should start")
        .block_on(async {
    let fixture = GitFixture::new();
    let tasks = TaskWorktreeService::new(fixture.storage.clone(), &fixture.worktrees_root);
    let plan = tasks
        .create_task(
            &fixture.repository_id,
            "Milestone B proof",
            "Create a proof artifact",
            "HEAD",
        )
        .expect("task should plan without mutating Git");
    assert_eq!(plan.state, WorktreeState::Planned);
    assert!(!plan.worktree_path.exists());

    let ready = tasks
        .provision(&plan.task_id)
        .expect("approved worktree should provision");
    assert_eq!(ready.state, WorktreeState::Ready);
    assert_eq!(
        git(&ready.worktree_path, &["branch", "--show-current"]).trim(),
        ready.branch_name,
    );

    let adapter = CodexAdapter;
    let mut request = AgentLaunchRequest::new(
        PathBuf::from("codex"),
        ready.worktree_path.clone(),
        concat!(
            "Create a file named mars-proof.txt containing exactly MARS_AGENT_WORKTREE_OK followed by a newline. ",
            "Then run git status --short to verify it. Do not modify any other file."
        ),
    );
    request.allow_write = true;
    let command = adapter
        .command(&request)
        .expect("Codex command should build");
    let mut spec = ProcessSpec::from_command("codex", command);
    spec.task_id = Some(plan.task_id.clone());
    let supervisor = ProcessSupervisor::new(fixture.storage.clone());
    let mut process = supervisor.launch(spec).expect("real agent should launch");
    let session_id = process.session_id.clone();
    let mut decoder = JsonLineDecoder::new();
    let mut normalized = Vec::new();
    let mut saw_exit = false;

    while let Some(event) = process.events.recv().await {
        match event.kind {
            ProcessEventKind::Stdout { text } => {
                normalized.extend(decoder.push(text.as_bytes(), &adapter));
            }
            ProcessEventKind::Exit { success, .. } => {
                saw_exit = true;
                normalized.extend(decoder.finish(&adapter));
                if !normalized
                    .iter()
                    .any(|event| matches!(event, NormalizedAgentEvent::End { .. }))
                {
                    normalized.push(adapter.on_exit(success.then_some(0).or(Some(1))));
                }
            }
            _ => {}
        }
    }

    assert!(
        saw_exit,
        "supervisor should stream a terminal process event"
    );
    assert_eq!(
        supervisor
            .session(&session_id)
            .expect("session should remain queryable")
            .status,
        SessionStatus::Completed,
    );
    assert!(normalized.iter().any(
        |event| matches!(event, NormalizedAgentEvent::Checkpoint { session_id } if !session_id.is_empty())
    ));
            assert!(normalized.iter().any(
                |event| matches!(event, NormalizedAgentEvent::Command { command, .. } if command.contains("git status"))
            ));
            assert!(normalized.iter().any(
                |event| matches!(event, NormalizedAgentEvent::FileChange { path, .. } if path.ends_with("mars-proof.txt"))
            ));
    assert!(
        normalized
            .iter()
            .any(|event| matches!(event, NormalizedAgentEvent::End { success: true, .. }))
    );
    assert_eq!(
        fs::read_to_string(ready.worktree_path.join("mars-proof.txt"))
            .expect("changed file should reopen"),
        "MARS_AGENT_WORKTREE_OK\n",
    );
            assert!(
                git(&ready.worktree_path, &["status", "--short"]).contains("mars-proof.txt")
            );
        });
}

struct GitFixture {
    _directory: TestDirectory,
    repository_id: String,
    storage: Arc<Storage>,
    worktrees_root: PathBuf,
}

impl GitFixture {
    fn new() -> Self {
        let directory = TestDirectory::new();
        let repository_root = directory.path().join("real agent repository");
        let worktrees_root = directory.path().join("Mars worktrees");
        fs::create_dir_all(&repository_root).expect("repository should exist");
        fs::create_dir_all(&worktrees_root).expect("worktree root should exist");
        git(
            &repository_root,
            &["init", "--quiet", "--initial-branch=main"],
        );
        fs::write(
            repository_root.join("README.md"),
            "# Mars real-agent proof\n",
        )
        .expect("fixture source should write");
        git(&repository_root, &["add", "README.md"]);
        git(
            &repository_root,
            &[
                "-c",
                "user.name=Mars Proof",
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
            _directory: directory,
            repository_id: project.repository_id,
            storage,
            worktrees_root,
        }
    }
}

fn git(root: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .expect("system Git should launch");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr),
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
        let path = std::env::temp_dir().join(format!(
            "mars-real-agent-proof-{}-{nonce}",
            std::process::id(),
        ));
        fs::create_dir_all(&path).expect("proof directory should exist");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if self.0.exists() {
            fs::remove_dir_all(&self.0).expect("proof directory should be removable");
        }
    }
}
