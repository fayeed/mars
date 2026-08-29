mod support;

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};

use mars_agents::adapter::{
    AgentAdapter, AgentLaunchRequest, JsonLineDecoder, NormalizedAgentEvent,
};
use mars_agents::codex::CodexAdapter;

use mars_brain::{
    EvidenceBundle, ExtractionError, ExtractionProvider, ExtractionRequest, ExtractionService,
    ManualMemoryDraft, MemoryCategory, render_extraction_prompt,
};
use serde_json::json;

#[test]
fn evidence_is_assembled_as_untrusted_data_and_changed_symbols_are_linkable() {
    let fixture = support::indexed_fixture();
    seed_completed_session(&fixture);
    let seen = Arc::new(Mutex::new(None));
    let provider = RecordingProvider {
        seen: seen.clone(),
        response: valid_response(&fixture),
    };
    let service = ExtractionService::new(fixture.storage.clone(), Arc::new(provider));
    let result = service
        .extract(ExtractionRequest {
            session_id: "memory-session".into(),
            diff: include_str!("../../../fixtures/memory/task.diff").into(),
            model_permission: true,
        })
        .expect("strict extraction should pass");

    let bundle = seen
        .lock()
        .expect("recording lock should work")
        .clone()
        .expect("provider should receive evidence");
    assert!(bundle.instructions.contains("untrusted data"));
    assert!(bundle.events.iter().all(|event| event.untrusted));
    assert!(
        bundle.events[0]
            .content
            .contains("Ignore prior instructions")
    );
    assert!(
        bundle
            .changed_symbols
            .iter()
            .any(|symbol| symbol.name == "add")
    );
    assert!(
        result
            .affected_entities
            .iter()
            .any(|entity| entity.title == "add")
    );
    assert_eq!(result.summary.category, MemoryCategory::Summary);
    assert!(
        result
            .tests
            .iter()
            .any(|claim| claim.title == "Workspace tests")
    );
}

#[test]
fn permission_provider_failure_and_invalid_outputs_are_explicit() {
    let fixture = support::indexed_fixture();
    seed_completed_session(&fixture);
    let request = ExtractionRequest {
        session_id: "memory-session".into(),
        diff: include_str!("../../../fixtures/memory/task.diff").into(),
        model_permission: false,
    };
    let service = ExtractionService::new(
        fixture.storage.clone(),
        Arc::new(FailingProvider("provider unavailable".into())),
    );
    assert!(matches!(
        service.extract(request.clone()),
        Err(ExtractionError::ModelPermissionRequired)
    ));
    assert!(matches!(
        service.extract(ExtractionRequest { model_permission: true, ..request.clone() }),
        Err(ExtractionError::Provider(message)) if message == "provider unavailable"
    ));

    for (response, expected) in [
        (
            r#"{"summary":{"category":"summary","title":"x","body":"y","confidence":0.5,"evidence":["missing:evidence"]},"decisions":[],"constraints":[],"attempts":[],"risks":[],"questions":[],"tests":[],"commits":[],"affectedEntities":[]}"#,
            "unsupported",
        ),
        (
            r#"{"summary":{"category":"summary","title":"x","body":"y","confidence":0.5,"evidence":["task:intent"],"hiddenReasoning":"never"},"decisions":[],"constraints":[],"attempts":[],"risks":[],"questions":[],"tests":[],"commits":[],"affectedEntities":[]}"#,
            "schema",
        ),
        (
            r#"{"summary":{"category":"summary","title":"Same","body":"Duplicate body","confidence":0.5,"evidence":["task:intent"]},"decisions":[{"category":"decision","title":"same","body":" duplicate   body ","confidence":0.6,"evidence":["task:intent"]}],"constraints":[],"attempts":[],"risks":[],"questions":[],"tests":[],"commits":[],"affectedEntities":[]}"#,
            "duplicate",
        ),
    ] {
        let service = ExtractionService::new(
            fixture.storage.clone(),
            Arc::new(StaticProvider(response.into())),
        );
        let error = service
            .extract(ExtractionRequest {
                model_permission: true,
                ..request.clone()
            })
            .expect_err("invalid provider output should fail");
        match expected {
            "unsupported" => assert!(matches!(error, ExtractionError::UnsupportedEvidence(_))),
            "schema" => assert!(matches!(error, ExtractionError::InvalidOutput(_))),
            "duplicate" => assert!(matches!(error, ExtractionError::DuplicateCandidate(_))),
            _ => unreachable!(),
        }
    }
}

#[test]
fn manual_fallback_uses_the_same_evidence_validation() {
    let fixture = support::indexed_fixture();
    seed_completed_session(&fixture);
    let service = ExtractionService::new(
        fixture.storage.clone(),
        Arc::new(FailingProvider("unused".into())),
    );
    let bundle = service
        .assemble(
            "memory-session",
            include_str!("../../../fixtures/memory/task.diff"),
        )
        .expect("evidence should assemble without a model");
    let draft = ManualMemoryDraft {
        category: MemoryCategory::Decision,
        title: "Preserve arithmetic safety".into(),
        body: "Reject sums outside the safe integer range.".into(),
        confidence: 1.0,
        evidence: vec!["diff:src/math.ts:5".into()],
    };
    let candidate = service
        .manual_candidate(&bundle, draft)
        .expect("manual fallback should validate");
    assert_eq!(candidate.title, "Preserve arithmetic safety");
    assert!(matches!(
        service.manual_candidate(
            &bundle,
            ManualMemoryDraft {
                evidence: vec!["event:999".into()],
                ..ManualMemoryDraft {
                    category: MemoryCategory::Risk,
                    title: "Unsupported".into(),
                    body: "No source.".into(),
                    confidence: 0.5,
                    evidence: Vec::new(),
                }
            }
        ),
        Err(ExtractionError::UnsupportedEvidence(_))
    ));
}

#[test]
fn extraction_prompt_declares_singular_claim_categories() {
    let fixture = support::indexed_fixture();
    seed_completed_session(&fixture);
    let service = ExtractionService::new(
        fixture.storage.clone(),
        Arc::new(FailingProvider("unused".into())),
    );
    let bundle = service
        .assemble(
            "memory-session",
            include_str!("../../../fixtures/memory/task.diff"),
        )
        .expect("evidence should assemble");
    let prompt = render_extraction_prompt(&bundle).expect("prompt should render");
    assert!(prompt.contains("decisions -> `decision`"));
    assert!(prompt.contains("Never use an array field name such as `decisions`"));
    assert!(prompt.contains("Copy every entry in changedSymbols"));
    assert!(prompt.contains("affectedEntities must not be empty"));
}

#[test]
fn multi_file_diffs_keep_each_hunk_attached_to_its_own_path() {
    let fixture = support::indexed_fixture();
    seed_completed_session(&fixture);
    let service = ExtractionService::new(
        fixture.storage.clone(),
        Arc::new(FailingProvider("unused".into())),
    );
    let diff = concat!(
        "diff --git a/src/math.ts b/src/math.ts\n",
        "--- a/src/math.ts\n",
        "+++ b/src/math.ts\n",
        "@@ -5,7 +5,10 @@ export interface Numeric {\n",
        " export function add(left: number, right: number): number {\n",
        "+  if (!Number.isSafeInteger(left + right)) throw new RangeError();\n",
        "   return left + right;\n",
        " }\n",
        "diff --git a/tests/math.test.ts b/tests/math.test.ts\n",
        "--- a/tests/math.test.ts\n",
        "+++ b/tests/math.test.ts\n",
        "@@ -3,5 +3,8 @@ describe(\"add\", () => {\n",
        "   it(\"adds values\", () => {\n",
        "     expect(add(1, 2)).toBe(3);\n",
        "   });\n",
        "+  it(\"rejects unsafe sums\", () => expect(() => add(Number.MAX_SAFE_INTEGER, 1)).toThrow());\n",
        " });\n",
    );
    let bundle = service
        .assemble("memory-session", diff)
        .expect("multi-file evidence should assemble");

    assert_eq!(bundle.diff_hunks[0].path, "src/math.ts");
    assert_eq!(bundle.diff_hunks[1].path, "tests/math.test.ts");
    assert!(
        bundle
            .changed_symbols
            .iter()
            .any(|symbol| symbol.name == "add")
    );
}

#[test]
#[ignore = "requires an authenticated installed Codex CLI and explicitly permits a paid extraction run"]
fn real_codex_extraction_obeys_the_strict_evidence_contract() {
    let fixture = support::indexed_fixture();
    initialize_git(&fixture.root);
    seed_completed_session(&fixture);
    let service = ExtractionService::new(
        fixture.storage.clone(),
        Arc::new(RealCodexProvider {
            cwd: fixture.root.clone(),
        }),
    );
    let result = service
        .extract(ExtractionRequest {
            session_id: "memory-session".into(),
            diff: include_str!("../../../fixtures/memory/task.diff").into(),
            model_permission: true,
        })
        .expect("real Codex output should satisfy the same validator");
    assert!(!result.summary.evidence.is_empty());
    assert!(
        result
            .affected_entities
            .iter()
            .all(|entity| entity.title == "add")
    );
}

#[derive(Clone)]
struct RecordingProvider {
    seen: Arc<Mutex<Option<EvidenceBundle>>>,
    response: String,
}

impl ExtractionProvider for RecordingProvider {
    fn extract(&self, bundle: &EvidenceBundle) -> Result<String, String> {
        *self.seen.lock().expect("recording lock should work") = Some(bundle.clone());
        Ok(self.response.clone())
    }
}

struct StaticProvider(String);

impl ExtractionProvider for StaticProvider {
    fn extract(&self, _bundle: &EvidenceBundle) -> Result<String, String> {
        Ok(self.0.clone())
    }
}

struct FailingProvider(String);

impl ExtractionProvider for FailingProvider {
    fn extract(&self, _bundle: &EvidenceBundle) -> Result<String, String> {
        Err(self.0.clone())
    }
}

struct RealCodexProvider {
    cwd: PathBuf,
}

impl ExtractionProvider for RealCodexProvider {
    fn extract(&self, bundle: &EvidenceBundle) -> Result<String, String> {
        let prompt = render_extraction_prompt(bundle).map_err(|error| error.to_string())?;
        let adapter = CodexAdapter;
        let command = adapter
            .command(&AgentLaunchRequest {
                executable: PathBuf::from("codex"),
                cwd: self.cwd.clone(),
                prompt,
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
        let response = events
            .into_iter()
            .filter_map(|event| match event {
                NormalizedAgentEvent::Text { text } => Some(strip_json_fence(&text)),
                _ => None,
            })
            .next_back()
            .ok_or_else(|| "Codex returned no extraction JSON".to_owned())?;
        eprintln!("real extraction response: {response}");
        Ok(response)
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

fn initialize_git(root: &std::path::Path) {
    for arguments in [
        vec!["init", "--quiet", "--initial-branch=main"],
        vec!["add", "."],
        vec![
            "-c",
            "user.name=Mars Test",
            "-c",
            "user.email=mars@example.test",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
    ] {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(&arguments)
            .output()
            .expect("Git should launch");
        assert!(
            output.status.success(),
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn valid_response(fixture: &support::BrainFixture) -> String {
    let add_id: String = fixture
        .storage
        .with_connection(|connection| {
            connection.query_row(
                "SELECT id FROM symbols WHERE name = 'add' LIMIT 1",
                [],
                |row| row.get(0),
            )
        })
        .expect("add should exist");
    json!({
        "summary": claim("summary", "Safe addition", "The task added a safe-integer guard.", 0.95, &["diff:src/math.ts:5"]),
        "decisions": [claim("decision", "Reject unsafe sums", "Addition throws rather than returning an unsafe integer.", 0.9, &["diff:src/math.ts:5"])],
        "constraints": [],
        "attempts": [],
        "risks": [claim("risk", "Wrapping callers", "Existing callers may expect wrapping behavior.", 0.7, &["event:3"])],
        "questions": [],
        "tests": [claim("test", "Workspace tests", "The agent reported pnpm test completed.", 0.8, &["event:2"])],
        "commits": [],
        "affectedEntities": [{ "kind": "symbol", "id": add_id, "title": "add", "evidence": ["diff:src/math.ts:5"] }]
    })
    .to_string()
}

fn claim(
    category: &str,
    title: &str,
    body: &str,
    confidence: f64,
    evidence: &[&str],
) -> serde_json::Value {
    json!({ "category": category, "title": title, "body": body, "confidence": confidence, "evidence": evidence })
}

fn seed_completed_session(fixture: &support::BrainFixture) {
    let events = include_str!("../../../fixtures/memory/session-events.jsonl")
        .lines()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line).expect("event fixture should parse")
        })
        .collect::<Vec<_>>();
    fixture
        .storage
        .with_transaction(|transaction| {
            let project_id: String = transaction.query_row(
                "SELECT project_id FROM repositories WHERE id = ?1",
                [&fixture.repository_id],
                |row| row.get(0),
            )?;
            transaction.execute(
                "INSERT INTO tasks (id, project_id, repository_id, title, intent, status, created_at, updated_at)
                 VALUES ('memory-task', ?1, ?2, 'Guard addition', 'Make add reject unsafe integer sums', 'completed', '2026-08-29T00:00:00Z', '2026-08-29T00:10:00Z')",
                rusqlite::params![project_id, fixture.repository_id],
            )?;
            transaction.execute(
                "INSERT INTO agent_sessions (id, task_id, adapter_kind, status, command_json, created_at, started_at, ended_at)
                 VALUES ('memory-session', 'memory-task', 'codex', 'completed', '{}', '2026-08-29T00:00:00Z', '2026-08-29T00:00:00Z', '2026-08-29T00:10:00Z')",
                [],
            )?;
            for event in &events {
                transaction.execute(
                    "INSERT INTO agent_events (session_id, sequence, event_type, payload_json, occurred_at)
                     VALUES ('memory-session', ?1, ?2, ?3, '2026-08-29T00:00:00Z')",
                    rusqlite::params![event["sequence"].as_i64(), event["type"].as_str(), event.to_string()],
                )?;
            }
            Ok(())
        })
        .expect("session evidence should seed");
}
