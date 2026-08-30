#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mars_agents::adapter::{
    AgentAdapter, AgentLaunchRequest, JsonLineDecoder, NormalizedAgentEvent,
};
use mars_agents::claude::ClaudeAdapter;
use mars_agents::codex::CodexAdapter;
use mars_agents::process::{ProcessEventKind, ProcessSpec, ProcessSupervisor, SessionStatus};
use mars_brain::{
    ContextPackRequest, ContextPackService, ReviewHandoffRequest, ReviewProvider, ReviewService,
    render_context_pack,
};
use mars_context::ContextService;
use mars_core::projects::{OpenRepositoryOutcome, ProjectService};
use mars_git::TaskWorktreeService;
use mars_security::TrustDecision;
use mars_storage::Storage;

#[test]
#[ignore = "requires authenticated installed Claude and Codex CLIs and explicitly permits two paid agent runs"]
fn claude_implements_and_codex_reviews_the_exact_shared_handoff() {
    let started = Instant::now();
    let fixture = RealFixture::new();
    let tasks = TaskWorktreeService::new(fixture.storage.clone(), &fixture.worktrees_root);
    let plan = tasks
        .create_task(
            &fixture.repository_id,
            "Add optional uppercase labels",
            "Add an uppercase option to formatLabel while preserving its existing whitespace behavior",
            "HEAD",
        )
        .expect("task should plan");
    let ready = tasks
        .provision(&plan.task_id)
        .expect("task worktree should provision");
    let context = ContextService::new(fixture.storage.clone(), &fixture.artifacts_root);
    let constraint = context
        .capture_text(
            &fixture.repository_id,
            Some(&plan.task_id),
            Some("Label whitespace contract"),
            "MARS_REVIEW_CONTEXT_814: formatLabel must trim leading and trailing whitespace before applying uppercase.",
        )
        .expect("shared constraint should capture");
    context
        .classify(&constraint.id, "constraint")
        .expect("shared context should become active constraint");
    let implementation_pack = ContextPackService::new(fixture.storage.clone())
        .build(ContextPackRequest {
            task_id: plan.task_id.clone(),
            adapter: "claude".into(),
            max_bytes: 24 * 1024,
            excluded_entity_refs: Vec::new(),
        })
        .expect("Claude context pack should build");
    assert!(
        implementation_pack
            .items
            .iter()
            .any(|item| item.content.contains("MARS_REVIEW_CONTEXT_814"))
    );
    let prompt = render_context_pack(
        &implementation_pack,
        concat!(
            "Implement the task in src/format.ts and add focused assertions to tests/format.test.ts. ",
            "Keep the public function name, make the smallest correct changes, run git diff --check, and do not commit."
        ),
    );

    let implementation_started = Instant::now();
    let claude = ClaudeAdapter;
    let command = claude
        .command(&AgentLaunchRequest {
            executable: PathBuf::from("claude"),
            cwd: ready.worktree_path.clone(),
            prompt,
            resume_session_id: None,
            allowed_tools: vec!["Read".into(), "Edit".into(), "Write".into(), "Bash".into()],
            allow_write: true,
        })
        .expect("Claude implementation command should build");
    let mut spec = ProcessSpec::new("claude", command.program, command.arguments, command.cwd);
    spec.task_id = Some(plan.task_id.clone());
    let supervisor = ProcessSupervisor::new(fixture.storage.clone());
    let implementation_session_id = tokio::runtime::Runtime::new()
        .expect("Tokio runtime should start")
        .block_on(async {
            let mut launched = supervisor
                .launch(spec)
                .expect("real Claude implementation should launch");
            let implementation_session_id = launched.session_id.clone();
            tokio::time::timeout(Duration::from_secs(300), async {
                let mut decoder = JsonLineDecoder::new();
                while let Some(process_event) = launched.events.recv().await {
                    let events = match process_event.kind {
                        ProcessEventKind::Stdout { text } => decoder.push(text.as_bytes(), &claude),
                        ProcessEventKind::Stderr { text } => vec![NormalizedAgentEvent::Unknown {
                            provider_type: "stderr".into(),
                            raw: serde_json::Value::String(text),
                        }],
                        ProcessEventKind::Exit { code, .. } => {
                            let mut events = decoder.finish(&claude);
                            if !events
                                .iter()
                                .any(|event| matches!(event, NormalizedAgentEvent::End { .. }))
                            {
                                events.push(claude.on_exit(code));
                            }
                            events
                        }
                        _ => Vec::new(),
                    };
                    for event in events {
                        supervisor
                            .record_normalized(&implementation_session_id, &event)
                            .expect("normalized implementation evidence should persist");
                    }
                }
            })
            .await
            .expect("Claude should finish within five minutes");
            implementation_session_id
        });
    assert_eq!(
        supervisor
            .session(&implementation_session_id)
            .expect("implementation session should persist")
            .status,
        SessionStatus::Completed
    );
    let implementation_elapsed = implementation_started.elapsed();
    let diff = git(
        &ready.worktree_path,
        &["diff", "--no-ext-diff", "--unified=3", "HEAD"],
    );
    assert!(diff.contains("toUpperCase"));
    assert!(diff.contains("trim"));

    let review_started = Instant::now();
    let reviews = ReviewService::new(fixture.storage.clone());
    let handoff = reviews
        .build_handoff(ReviewHandoffRequest {
            task_id: plan.task_id.clone(),
            implementation_session_id: implementation_session_id.clone(),
            reviewer_adapter: "codex".into(),
            diff: diff.clone(),
            max_bytes: 64 * 1024,
        })
        .expect("shared review handoff should build");
    assert_eq!(handoff.diff, diff);
    assert_eq!(handoff.diff_sha256.len(), 64);
    assert!(
        handoff
            .context_pack
            .items
            .iter()
            .any(|item| item.content.contains("MARS_REVIEW_CONTEXT_814"))
    );
    assert!(!handoff.implementation_events.is_empty());
    let review = reviews
        .run_review(
            &handoff,
            true,
            &CodexReviewProvider {
                cwd: ready.worktree_path,
            },
        )
        .expect("real Codex review should satisfy the strict evidence contract");
    assert!(!review.summary.trim().is_empty());
    assert_eq!(review.reviewer_adapter, "codex");
    assert_eq!(
        reviews
            .list_for_task(&plan.task_id)
            .expect("review should remain queryable"),
        vec![review.clone()]
    );
    eprintln!(
        "milestone-e proof: implementation_ms={}, review_ms={}, handoff_bytes={}, context_items={}, normalized_events={}, verdict={:?}, findings={}, total_ms={}",
        implementation_elapsed.as_millis(),
        review_started.elapsed().as_millis(),
        handoff.total_bytes,
        handoff.context_pack.items.len(),
        handoff.implementation_events.len(),
        review.verdict,
        review.findings.len(),
        started.elapsed().as_millis(),
    );
}

struct CodexReviewProvider {
    cwd: PathBuf,
}

impl ReviewProvider for CodexReviewProvider {
    fn review(&self, prompt: &str) -> Result<String, String> {
        let adapter = CodexAdapter;
        let command = adapter
            .command(&AgentLaunchRequest {
                executable: PathBuf::from("codex"),
                cwd: self.cwd.clone(),
                prompt: prompt.into(),
                resume_session_id: None,
                allowed_tools: Vec::new(),
                allow_write: false,
            })
            .map_err(|error| error.to_string())?;
        let output = Command::new(command.program)
            .args(command.arguments)
            .current_dir(command.cwd)
            .output()
            .map_err(|error| error.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).into_owned());
        }
        let mut decoder = JsonLineDecoder::new();
        let mut events = decoder.push(&output.stdout, &adapter);
        events.extend(decoder.finish(&adapter));
        events
            .into_iter()
            .filter_map(|event| match event {
                NormalizedAgentEvent::Text { text } => Some(strip_json_fence(&text)),
                _ => None,
            })
            .next_back()
            .ok_or_else(|| "Codex returned no review JSON".into())
    }
}

fn strip_json_fence(value: &str) -> String {
    let trimmed = value.trim();
    trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|value| value.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(trimmed)
        .to_owned()
}

struct RealFixture {
    _directory: TestDirectory,
    repository_id: String,
    storage: Arc<Storage>,
    artifacts_root: PathBuf,
    worktrees_root: PathBuf,
}

impl RealFixture {
    fn new() -> Self {
        let directory = TestDirectory::new();
        let repository_root = directory.path().join("cross agent repository");
        fs::create_dir_all(repository_root.join("src")).expect("source directory should exist");
        fs::create_dir_all(repository_root.join("tests")).expect("test directory should exist");
        fs::write(
            repository_root.join("src/format.ts"),
            "export function formatLabel(value: string): string {\n  return value.trim();\n}\n",
        )
        .expect("source fixture should write");
        fs::write(
            repository_root.join("tests/format.test.ts"),
            "import { formatLabel } from '../src/format';\n\nconsole.assert(formatLabel(' Mars ') === 'Mars');\n",
        )
        .expect("test fixture should write");
        git(
            &repository_root,
            &["init", "--quiet", "--initial-branch=main"],
        );
        git(&repository_root, &["add", "."]);
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
                "fixture",
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
            repository_id: project.repository_id,
            storage,
            artifacts_root: directory.path().join("artifacts"),
            worktrees_root: directory.path().join("worktrees"),
            _directory: directory,
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
        let path = std::env::temp_dir().join(format!(
            "mars-cross-agent-proof-{}-{nonce}",
            std::process::id()
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
