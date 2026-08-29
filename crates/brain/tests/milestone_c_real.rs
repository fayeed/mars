#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mars_agents::adapter::{
    AgentAdapter, AgentLaunchRequest, JsonLineDecoder, NormalizedAgentEvent,
};
use mars_agents::codex::CodexAdapter;
use mars_agents::process::{ProcessEventKind, ProcessSpec, ProcessSupervisor, SessionStatus};
use mars_brain::{ContextPackRequest, ContextPackService, render_context_pack};
use mars_context::ContextService;
use mars_core::projects::{OpenRepositoryOutcome, ProjectService};
use mars_git::TaskWorktreeService;
use mars_security::TrustDecision;
use mars_skills::{SkillScope, SkillService, SkillsShRegistry};
use mars_storage::Storage;

#[test]
#[ignore = "requires skills.sh network access plus an authenticated installed Codex CLI and permits a paid agent run"]
fn captured_text_image_and_real_skill_reach_a_real_agent() {
    let fixture = RealFixture::new();
    let tasks = TaskWorktreeService::new(fixture.storage.clone(), &fixture.worktrees_root);
    let plan = tasks
                .create_task(
                    &fixture.repository_id,
                    "Milestone C real context proof",
                    "Read the supplied text, image reference, and web design skill, then write the proof file",
                    "HEAD",
                )
                .expect("task should plan");
    let ready = tasks
        .provision(&plan.task_id)
        .expect("worktree should provision");

    let context = ContextService::new(fixture.storage.clone(), &fixture.artifacts_root);
    let text = context
        .capture_text(
            &fixture.repository_id,
            Some(&plan.task_id),
            Some("Copper proof constraint"),
            "The required proof token is ORBIT-COPPER-731.",
        )
        .expect("text context should capture");
    context
        .classify(&text.id, "constraint")
        .expect("text should become an active constraint");
    context
        .capture_bytes(
            &fixture.repository_id,
            Some(&plan.task_id),
            Some("Mars context pixel"),
            "mars-context-pixel.png",
            ONE_PIXEL_PNG,
            "image/png",
        )
        .expect("image context should preserve and attach");

    let skills = SkillService::new(
        fixture.storage.clone(),
        Arc::new(SkillsShRegistry::new().expect("skills.sh client should build")),
        &fixture.skills_root,
    );
    let candidate = skills
        .search("web design guidelines")
        .expect("real skills.sh search should succeed")
        .into_iter()
        .find(|candidate| candidate.name == "web-design-guidelines")
        .expect("known public skill should be discoverable");
    let inspected = skills
        .inspect(&candidate)
        .expect("real skill should inspect");
    let installed = skills
        .trust_and_install(
            &inspected.id,
            Some(&fixture.repository_id),
            Some(&plan.task_id),
            SkillScope::Task,
            false,
            true,
        )
        .expect("inspected skill should install for Codex");

    let pack = ContextPackService::new(fixture.storage.clone())
        .build(ContextPackRequest {
            task_id: plan.task_id.clone(),
            adapter: "codex".into(),
            max_bytes: 24 * 1024,
            excluded_entity_refs: Vec::new(),
        })
        .expect("context pack should build");
    assert!(
        pack.items
            .iter()
            .any(|item| item.title == "Copper proof constraint")
    );
    assert!(
        pack.items
            .iter()
            .any(|item| item.title == "Mars context pixel")
    );
    assert!(
        pack.items
            .iter()
            .any(|item| item.title == "web-design-guidelines")
    );
    let prompt = render_context_pack(
        &pack,
        concat!(
            "Create context-proof.txt with exactly these three lines:\n",
            "TEXT=the proof token found in the supplied human constraint\n",
            "IMAGE=the exact title of the supplied image\n",
            "SKILL=the exact title of the supplied skill\n",
            "Resolve each value from the context pack. Do not modify any other file."
        ),
    );

    let adapter = CodexAdapter;
    let command = adapter
        .command(&AgentLaunchRequest {
            executable: PathBuf::from("codex"),
            cwd: ready.worktree_path.clone(),
            prompt,
            resume_session_id: None,
            allowed_tools: Vec::new(),
            allow_write: true,
        })
        .expect("Codex command should build");
    let mut spec = ProcessSpec::new("codex", command.program, command.arguments, command.cwd);
    spec.task_id = Some(plan.task_id.clone());
    let supervisor = ProcessSupervisor::new(fixture.storage.clone());
    let (normalized, session_id) = tokio::runtime::Runtime::new()
        .expect("Tokio runtime should start")
        .block_on(async {
            let mut process = supervisor
                .launch(spec)
                .expect("real Codex agent should launch");
            let session_id = process.session_id.clone();
            let normalized = tokio::time::timeout(Duration::from_secs(300), async {
                let mut decoder = JsonLineDecoder::new();
                let mut normalized = Vec::new();
                while let Some(event) = process.events.recv().await {
                    match event.kind {
                        ProcessEventKind::Stdout { text } => {
                            normalized.extend(decoder.push(text.as_bytes(), &adapter));
                        }
                        ProcessEventKind::Exit { code, .. } => {
                            normalized.extend(decoder.finish(&adapter));
                            if !normalized
                                .iter()
                                .any(|event| matches!(event, NormalizedAgentEvent::End { .. }))
                            {
                                normalized.push(adapter.on_exit(code));
                            }
                        }
                        _ => {}
                    }
                }
                normalized
            })
            .await
            .expect("real agent should finish within five minutes");

            (normalized, session_id)
        });

    assert_eq!(
        supervisor
            .session(&session_id)
            .expect("session should persist")
            .status,
        SessionStatus::Completed,
    );
    assert!(
        normalized
            .iter()
            .any(|event| matches!(event, NormalizedAgentEvent::End { success: true, .. }))
    );
    let proof = fs::read_to_string(ready.worktree_path.join("context-proof.txt"))
        .expect("agent proof should reopen");
    assert_eq!(
        proof,
        "TEXT=ORBIT-COPPER-731\nIMAGE=Mars context pixel\nSKILL=web-design-guidelines\n"
    );
    eprintln!(
        "milestone-c proof: pack_bytes={}, items={}, exclusions={}, skill_hash={}",
        pack.total_bytes,
        pack.items.len(),
        pack.exclusions.len(),
        installed.pinned_hash,
    );
}

const ONE_PIXEL_PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 8, 215, 99, 248, 207, 192, 240, 31, 0, 5,
    0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

struct RealFixture {
    _directory: TestDirectory,
    repository_id: String,
    storage: Arc<Storage>,
    artifacts_root: PathBuf,
    skills_root: PathBuf,
    worktrees_root: PathBuf,
}

impl RealFixture {
    fn new() -> Self {
        let directory = TestDirectory::new();
        let repository_root = directory.path().join("context proof repository");
        fs::create_dir_all(&repository_root).expect("repository should exist");
        git(
            &repository_root,
            &["init", "--quiet", "--initial-branch=main"],
        );
        fs::write(repository_root.join("README.md"), "# Context proof\n")
            .expect("fixture should write");
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
            repository_id: project.repository_id,
            storage,
            artifacts_root: directory.path().join("artifacts"),
            skills_root: directory.path().join("skills"),
            worktrees_root: directory.path().join("worktrees"),
            _directory: directory,
        }
    }
}

fn git(root: &Path, arguments: &[&str]) {
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
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow Unix epoch")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("mars-context-proof-{}-{nonce}", std::process::id()));
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
