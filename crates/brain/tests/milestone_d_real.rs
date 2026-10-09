#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mars_agents::adapter::{
    AgentAdapter, AgentLaunchRequest, JsonLineDecoder, NormalizedAgentEvent,
};
use mars_agents::codex::CodexAdapter;
use mars_agents::process::{ProcessEventKind, ProcessSpec, ProcessSupervisor, SessionStatus};
use mars_brain::{
    AffectedEntityDraft, ClaimDraft, EntityKind, EntityRef, EvidenceBundle, ExtractionProvider,
    ExtractionRequest, ExtractionService, KnowledgeKind, KnowledgeService, NewKnowledge,
    NewProvenance, ProvenanceKind, WhyRequest, WhyService, WhyStatus, render_extraction_prompt,
};
use mars_core::projects::{OpenRepositoryOutcome, ProjectService};
use mars_git::TaskWorktreeService;
use mars_indexer::Indexer;
use mars_security::TrustDecision;
use mars_storage::Storage;
use tokio_util::sync::CancellationToken;

#[test]
#[ignore = "requires an authenticated installed Codex CLI and explicitly permits two paid agent runs"]
fn real_agent_work_becomes_human_reviewed_explainable_memory() {
    let started = Instant::now();
    let fixture = RealFixture::new();
    let tasks = TaskWorktreeService::new(fixture.storage.clone(), &fixture.worktrees_root);
    let plan = tasks
        .create_task(
            &fixture.repository_id,
            "Guard unsafe integer addition",
            "Make add reject unsafe integer sums and cover the behavior with a focused test",
            "HEAD",
        )
        .expect("task should plan");
    let ready = tasks
        .provision(&plan.task_id)
        .expect("worktree should provision");

    let implementation_started = Instant::now();
    let adapter = CodexAdapter;
    let command = adapter
        .command(&AgentLaunchRequest {
            executable: PathBuf::from("codex"),
            cwd: ready.worktree_path.clone(),
            prompt: concat!(
                "Implement this task in the current repository: make the exported add function in src/math.ts ",
                "throw RangeError when its result is not a safe integer, while preserving normal addition. ",
                "Add a focused test in tests/math.test.ts. Inspect the existing files, make only those necessary edits, ",
                "and run git diff --check. Do not commit."
            )
            .into(),
            resume_session_id: None,
            allowed_tools: Vec::new(),
            allow_write: true,
                    ..Default::default()
        })
        .expect("Codex implementation command should build");
    let mut spec = ProcessSpec::from_command("codex", command);
    spec.task_id = Some(plan.task_id.clone());
    let supervisor = ProcessSupervisor::new(fixture.storage.clone());
    let session_id = tokio::runtime::Runtime::new()
        .expect("Tokio runtime should start")
        .block_on(async {
            let mut process = supervisor
                .launch(spec)
                .expect("real implementation agent should launch");
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
            .expect("implementation agent should finish within five minutes");
            assert!(
                normalized
                    .iter()
                    .any(|event| matches!(event, NormalizedAgentEvent::End { success: true, .. }))
            );
            session_id
        });
    assert_eq!(
        supervisor
            .session(&session_id)
            .expect("session should persist")
            .status,
        SessionStatus::Completed
    );
    let implementation_elapsed = implementation_started.elapsed();
    let diff = git(
        &ready.worktree_path,
        &["diff", "--no-ext-diff", "--unified=3", "HEAD"],
    );
    assert!(diff.contains("Number.isSafeInteger"));
    assert!(diff.contains("RangeError"));

    let extraction_started = Instant::now();
    let extraction = ExtractionService::new(
        fixture.storage.clone(),
        Arc::new(RealCodexProvider {
            cwd: ready.worktree_path.clone(),
        }),
    );
    let bundle = extraction
        .assemble(&session_id, &diff)
        .expect("live evidence should assemble before model extraction");
    assert!(
        bundle
            .changed_symbols
            .iter()
            .any(|symbol| symbol.name == "add"),
        "the diff must map to the indexed add symbol before extraction"
    );
    let extracted = extraction
        .extract(ExtractionRequest {
            session_id: session_id.clone(),
            diff,
            model_permission: true,
        })
        .expect("real extraction should satisfy the strict evidence contract");
    assert!(
        extracted
            .affected_entities
            .iter()
            .any(|entity| entity.title == "add")
    );
    let extracted_decision = extracted
        .decisions
        .first()
        .cloned()
        .expect("the deliberate unsafe-integer behavior should extract as a decision");
    let extraction_elapsed = extraction_started.elapsed();

    let project_id: String = fixture
        .storage
        .with_connection(|connection| {
            connection.query_row(
                "SELECT project_id FROM tasks WHERE id = ?1",
                [&plan.task_id],
                |row| row.get(0),
            )
        })
        .expect("task project should exist");
    let knowledge = KnowledgeService::new(fixture.storage.clone());
    let summary = persist_candidate(
        &knowledge,
        &project_id,
        &session_id,
        extracted.summary,
        &extracted.affected_entities,
    );
    let decision = persist_candidate(
        &knowledge,
        &project_id,
        &session_id,
        extracted_decision.clone(),
        &extracted.affected_entities,
    );
    let human = knowledge
        .create_provenance(NewProvenance {
            project_id: project_id.clone(),
            kind: ProvenanceKind::Human,
            source: EntityRef::new(EntityKind::Task, &plan.task_id),
            evidence: serde_json::json!({
                "visibleReview": true,
                "action": "approve and correct",
                "evidenceIds": extracted_decision.evidence,
            }),
            observed_at: "2026-08-29T00:00:00Z".into(),
        })
        .expect("human review provenance should persist");
    knowledge
        .approve(&summary.id, &human.id)
        .expect("human should approve the extracted summary");
    let approved_decision = knowledge
        .approve(&decision.id, &human.id)
        .expect("human should approve the extracted decision");
    let corrected = knowledge
        .correct(
            &approved_decision.id,
            "Reject unsafe integer sums",
            "The add function deliberately throws RangeError instead of returning an imprecise integer.",
            &human.id,
        )
        .expect("human correction should supersede without losing code links");

    let add_id = extracted
        .affected_entities
        .iter()
        .find(|entity| entity.title == "add")
        .expect("add should be linked")
        .id
        .clone();
    let answer = WhyService::new(fixture.storage.clone())
        .ask(WhyRequest {
            project_id,
            subject: EntityRef::new(EntityKind::Symbol, add_id),
            max_depth: 5,
            max_paths: 20,
        })
        .expect("why query should trace reviewed memory");
    assert_ne!(answer.status, WhyStatus::InsufficientEvidence);
    assert!(
        answer
            .facts
            .iter()
            .any(|fact| fact.knowledge_id == corrected.id && fact.provenance == "HUMAN")
    );
    assert!(
        answer
            .sources
            .iter()
            .any(|source| source.location == format!("agent_session:{session_id}"))
    );
    assert!(
        answer
            .sources
            .iter()
            .any(|source| source.location.starts_with("diff:src/math.ts:"))
    );
    assert!(answer.paths.iter().any(|path| {
        path.steps.iter().any(|step| step.kind == "agent_session")
            && path.steps.iter().any(|step| step.kind == "task")
            && path.steps.iter().any(|step| step.kind == "evidence")
    }));
    eprintln!(
        "milestone-d proof: implementation_ms={}, extraction_ms={}, candidates=2, facts={}, paths={}, total_ms={}",
        implementation_elapsed.as_millis(),
        extraction_elapsed.as_millis(),
        answer.facts.len(),
        answer.paths.len(),
        started.elapsed().as_millis(),
    );
}

fn persist_candidate(
    knowledge: &KnowledgeService,
    project_id: &str,
    session_id: &str,
    claim: ClaimDraft,
    affected: &[AffectedEntityDraft],
) -> mars_brain::KnowledgeRecord {
    let provenance = knowledge
        .create_provenance(NewProvenance {
            project_id: project_id.into(),
            kind: ProvenanceKind::AgentInferred,
            source: EntityRef::new(EntityKind::AgentSession, session_id),
            evidence: serde_json::json!({ "evidenceIds": claim.evidence }),
            observed_at: "2026-08-29T00:00:00Z".into(),
        })
        .expect("agent provenance should persist");
    let record = knowledge
        .create_candidate(NewKnowledge {
            project_id: project_id.into(),
            kind: match claim.category {
                mars_brain::MemoryCategory::Summary => KnowledgeKind::Summary,
                mars_brain::MemoryCategory::Decision => KnowledgeKind::Decision,
                mars_brain::MemoryCategory::Constraint => KnowledgeKind::Constraint,
                mars_brain::MemoryCategory::Attempt => KnowledgeKind::Attempt,
                mars_brain::MemoryCategory::Risk => KnowledgeKind::Risk,
                mars_brain::MemoryCategory::Question => KnowledgeKind::Question,
                mars_brain::MemoryCategory::Test => KnowledgeKind::Test,
                mars_brain::MemoryCategory::Commit => KnowledgeKind::Commit,
            },
            title: claim.title,
            body: claim.body,
            confidence: claim.confidence,
            provenance_id: provenance.id.clone(),
        })
        .expect("candidate should persist");
    for entity in affected {
        knowledge
            .link(
                project_id,
                EntityRef::new(EntityKind::Knowledge, &record.id),
                "AFFECTS",
                EntityRef::new(entity.kind, &entity.id),
                &provenance.id,
                claim.confidence,
            )
            .expect("candidate should link to changed code");
    }
    record
}

struct RealCodexProvider {
    cwd: PathBuf,
}

impl ExtractionProvider for RealCodexProvider {
    fn extract(&self, bundle: &EvidenceBundle) -> Result<String, String> {
        let adapter = CodexAdapter;
        let command = adapter
            .command(&AgentLaunchRequest {
                executable: PathBuf::from("codex"),
                cwd: self.cwd.clone(),
                prompt: render_extraction_prompt(bundle).map_err(|error| error.to_string())?,
                resume_session_id: None,
                allowed_tools: Vec::new(),
                allow_write: false,
                ..Default::default()
            })
            .map_err(|error| error.to_string())?;
        let output = command.output(&[]).map_err(|error| error.to_string())?;
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
            .ok_or_else(|| "Codex returned no extraction JSON".into())
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
    worktrees_root: PathBuf,
}

impl RealFixture {
    fn new() -> Self {
        let directory = TestDirectory::new();
        let repository_root = directory.path().join("memory proof repository");
        copy_directory(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/repositories/typescript-graph"),
            &repository_root,
        );
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
        Indexer::new(storage.clone())
            .index_repository(&project.repository_id, &CancellationToken::new())
            .expect("repository should index before agent work");
        Self {
            repository_id: project.repository_id,
            storage,
            worktrees_root: directory.path().join("worktrees"),
            _directory: directory,
        }
    }
}

fn copy_directory(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("destination should exist");
    for entry in fs::read_dir(source).expect("source should list") {
        let entry = entry.expect("source entry should load");
        let target = destination.join(entry.file_name());
        if entry.file_type().expect("entry type should load").is_dir() {
            copy_directory(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("fixture file should copy");
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
        let path =
            std::env::temp_dir().join(format!("mars-memory-proof-{}-{nonce}", std::process::id()));
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
