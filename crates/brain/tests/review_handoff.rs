mod support;

use mars_brain::{
    FindingStatus, ReviewError, ReviewHandoff, ReviewHandoffRequest, ReviewProvider, ReviewService,
    ReviewVerdict,
};
use rusqlite::params;
use serde_json::json;

#[test]
fn handoff_is_cross_agent_exact_diff_compact_and_provider_neutral() {
    let fixture = support::indexed_fixture();
    seed_completed_implementation(&fixture);
    let service = ReviewService::new(fixture.storage.clone());
    let diff = "diff --git a/src/math.ts b/src/math.ts\n--- a/src/math.ts\n+++ b/src/math.ts\n@@ -7,1 +7,2 @@\n+  if (!Number.isSafeInteger(left + right)) throw new RangeError();\n";
    let handoff = service
        .build_handoff(ReviewHandoffRequest {
            task_id: "review-task".into(),
            implementation_session_id: "claude-implementation".into(),
            reviewer_adapter: "codex".into(),
            diff: diff.into(),
            max_bytes: 32 * 1024,
        })
        .expect("cross-agent handoff should build");

    assert_eq!(handoff.implementation_adapter, "claude");
    assert_eq!(handoff.reviewer_adapter, "codex");
    assert_eq!(handoff.diff, diff);
    assert_eq!(handoff.diff_sha256.len(), 64);
    assert!(handoff.total_bytes <= handoff.max_bytes);
    assert_eq!(
        serde_json::to_vec(&handoff)
            .expect("handoff should serialize")
            .len(),
        handoff.total_bytes
    );
    assert!(
        handoff
            .implementation_events
            .iter()
            .any(|event| event.event_type == "text" && event.summary.contains("safe integer"))
    );
    assert!(
        handoff
            .implementation_events
            .iter()
            .any(|event| event.event_type == "test" && event.summary == "pnpm test")
    );
    let encoded = serde_json::to_string(&handoff).expect("handoff should encode");
    assert!(!encoded.contains("provider_raw_frame"));
    assert!(handoff.implementation_events.iter().all(|event| {
        event
            .source_location
            .starts_with("agent_session:claude-implementation")
    }));
}

#[test]
fn handoff_rejects_same_adapter_unfinished_sessions_and_truncated_diffs() {
    let fixture = support::indexed_fixture();
    seed_completed_implementation(&fixture);
    let service = ReviewService::new(fixture.storage.clone());
    let request = ReviewHandoffRequest {
        task_id: "review-task".into(),
        implementation_session_id: "claude-implementation".into(),
        reviewer_adapter: "claude".into(),
        diff: "diff".into(),
        max_bytes: 4 * 1024,
    };
    assert!(matches!(
        service.build_handoff(request),
        Err(ReviewError::SameAdapter)
    ));
    assert!(matches!(
        service.build_handoff(ReviewHandoffRequest {
            task_id: "review-task".into(),
            implementation_session_id: "missing".into(),
            reviewer_adapter: "codex".into(),
            diff: "diff".into(),
            max_bytes: 4 * 1024,
        }),
        Err(ReviewError::SessionNotReady(_))
    ));
    assert!(matches!(
        service.build_handoff(ReviewHandoffRequest {
            task_id: "review-task".into(),
            implementation_session_id: "claude-implementation".into(),
            reviewer_adapter: "codex".into(),
            diff: "x".repeat(3 * 1024),
            max_bytes: 4 * 1024,
        }),
        Err(ReviewError::ExactDiffTooLarge)
    ));
}

#[test]
fn strict_review_persists_evidence_backed_findings_and_builds_a_followup() {
    let fixture = support::indexed_fixture();
    seed_completed_implementation(&fixture);
    let service = ReviewService::new(fixture.storage.clone());
    let handoff = handoff(&service);
    let provider = StaticReview(
        json!({
            "verdict": "changes_requested",
            "summary": "The overflow guard is incomplete.",
            "findings": [{
                "severity": "error",
                "title": "Inputs are not validated",
                "body": "An already unsafe operand can lose precision before the result check.",
                "relativePath": "src/math.ts",
                "line": 8,
                "evidence": ["diff:src/math.ts:7"]
            }]
        })
        .to_string(),
    );
    let review = service
        .run_review(&handoff, true, &provider)
        .expect("strict review should persist atomically");

    assert_eq!(review.verdict, ReviewVerdict::ChangesRequested);
    assert_eq!(review.findings.len(), 1);
    assert_eq!(review.findings[0].status, FindingStatus::Open);
    assert_eq!(
        service
            .list_for_task("review-task")
            .expect("reviews should list"),
        vec![review.clone()]
    );
    let followup = service
        .render_followup(&review.id)
        .expect("open finding should render for the implementer");
    assert!(followup.contains("Inputs are not validated"));
    assert!(followup.contains("src/math.ts:8"));
    assert!(followup.contains("diff:src/math.ts:7"));
    let sent = service
        .set_finding_status(&review.findings[0].id, FindingStatus::Sent)
        .expect("finding should mark sent");
    assert_eq!(sent.status, FindingStatus::Sent);
}

#[test]
fn strict_review_rejects_permission_schema_evidence_and_duplicates() {
    let fixture = support::indexed_fixture();
    seed_completed_implementation(&fixture);
    let service = ReviewService::new(fixture.storage.clone());
    let handoff = handoff(&service);
    assert!(matches!(
        service.run_review(&handoff, false, &StaticReview("{}".into())),
        Err(ReviewError::ModelPermissionRequired)
    ));

    for (response, expected) in [
        (
            json!({ "verdict": "pass", "summary": "Looks good", "findings": [], "hiddenReasoning": "never" }).to_string(),
            "schema",
        ),
        (
            json!({
                "verdict": "changes_requested", "summary": "Issue", "findings": [{
                    "severity": "error", "title": "Issue", "body": "Body",
                    "relativePath": "src/math.ts", "line": 8, "evidence": ["diff:missing.ts:1"]
                }]
            }).to_string(),
            "evidence",
        ),
        (
            json!({
                "verdict": "changes_requested", "summary": "Issue", "findings": [
                    { "severity": "error", "title": "Same", "body": "Body", "relativePath": "src/math.ts", "line": 8, "evidence": ["diff:src/math.ts:7"] },
                    { "severity": "error", "title": " same ", "body": " body ", "relativePath": "src/math.ts", "line": 8, "evidence": ["diff:src/math.ts:7"] }
                ]
            }).to_string(),
            "duplicate",
        ),
    ] {
        let error = service
            .run_review(&handoff, true, &StaticReview(response))
            .expect_err("invalid review must fail closed");
        match expected {
            "schema" => assert!(matches!(error, ReviewError::InvalidOutput(_))),
            "evidence" => assert!(matches!(error, ReviewError::UnsupportedEvidence(_))),
            "duplicate" => assert!(matches!(error, ReviewError::DuplicateFinding(_))),
            _ => unreachable!(),
        }
    }
    assert!(
        service
            .list_for_task("review-task")
            .expect("invalid reviews should not persist")
            .is_empty()
    );
}

fn handoff(service: &ReviewService) -> ReviewHandoff {
    service
        .build_handoff(ReviewHandoffRequest {
            task_id: "review-task".into(),
            implementation_session_id: "claude-implementation".into(),
            reviewer_adapter: "codex".into(),
            diff: concat!(
                "diff --git a/src/math.ts b/src/math.ts\n",
                "--- a/src/math.ts\n",
                "+++ b/src/math.ts\n",
                "@@ -7,1 +7,2 @@\n",
                "+  if (!Number.isSafeInteger(left + right)) throw new RangeError();\n",
            )
            .into(),
            max_bytes: 32 * 1024,
        })
        .expect("handoff should build")
}

struct StaticReview(String);

impl ReviewProvider for StaticReview {
    fn review(&self, _prompt: &str) -> Result<String, String> {
        Ok(self.0.clone())
    }
}

fn seed_completed_implementation(fixture: &support::BrainFixture) {
    fixture
        .storage
        .with_transaction(|transaction| {
            let project_id: String = transaction.query_row(
                "SELECT project_id FROM repositories WHERE id = ?1",
                [&fixture.repository_id],
                |row| row.get(0),
            )?;
            transaction.execute(
                "INSERT INTO tasks
                 (id, project_id, repository_id, title, intent, status, created_at, updated_at)
                 VALUES ('review-task', ?1, ?2, 'Guard addition', 'Review the implementation',
                         'review', '2026-08-29T00:00:00Z', '2026-08-29T00:10:00Z')",
                params![project_id, fixture.repository_id],
            )?;
            transaction.execute(
                "INSERT INTO agent_sessions
                 (id, task_id, adapter_kind, status, command_json, created_at, started_at, ended_at)
                 VALUES ('claude-implementation', 'review-task', 'claude', 'completed', '{}',
                         '2026-08-29T00:00:00Z', '2026-08-29T00:00:00Z', '2026-08-29T00:10:00Z')",
                [],
            )?;
            for (sequence, event_type, payload) in [
                (
                    1,
                    "text",
                    json!({ "type": "text", "text": "Implemented a safe integer guard." }),
                ),
                (
                    2,
                    "file_change",
                    json!({ "type": "file_change", "path": "src/math.ts", "change": "updated" }),
                ),
                (
                    3,
                    "test",
                    json!({ "type": "test", "command": "pnpm test", "status": "passed" }),
                ),
                (
                    4,
                    "end",
                    json!({ "type": "end", "success": true, "message": null }),
                ),
            ] {
                transaction.execute(
                    "INSERT INTO normalized_agent_events
                     (session_id, sequence, event_type, payload_json, occurred_at)
                     VALUES ('claude-implementation', ?1, ?2, ?3, '2026-08-29T00:00:00Z')",
                    params![sequence, event_type, payload.to_string()],
                )?;
            }
            transaction.execute(
                "INSERT INTO agent_events
                 (session_id, sequence, event_type, payload_json, occurred_at)
                 VALUES ('claude-implementation', 1, 'stdout', ?1, '2026-08-29T00:00:00Z')",
                [json!({ "type": "stdout", "text": "provider_raw_frame" }).to_string()],
            )?;
            Ok(())
        })
        .expect("implementation evidence should seed");
}
