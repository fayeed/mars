mod support;

use mars_activity::{ActivityEntry, ActivityKind, Actor, NewRun, RunRole};
use serde_json::json;

use support::ActivityFixture;

const WHEN: &str = "2026-09-09T09:00:00Z";

/// A task that exercises every part an audit is asked for: the transcript, the
/// commands, the diffs, and the decisions.
fn recorded() -> (ActivityFixture, String) {
    let fixture = ActivityFixture::new();
    let record = fixture.record();
    let runs = fixture.runs();
    let task = fixture.task("task-audit", "Guard the divisor");
    let run = runs
        .start(NewRun {
            task_id: task.clone(),
            agent_id: "claude-code".into(),
            adapter_kind: "claude".into(),
            role: RunRole::Implementer,
            capabilities: json!({ "writeFiles": true }),
            context_pack_digest: None,
        })
        .expect("run should start");

    for entry in [
        ActivityEntry::new(
            &task,
            ActivityKind::Message,
            Actor::Human,
            "fayeed",
            "Guard the divisor in divide()",
        )
        .with_payload(json!({ "text": "Guard the divisor in divide()." })),
        ActivityEntry::new(
            &task,
            ActivityKind::WorktreeCreated,
            Actor::System,
            "mars",
            "worktree ready",
        )
        .with_payload(json!({
            "branchName": "mars/guard-divisor",
            "baseRef": "main",
            "baseOid": "abc123def456",
            "path": fixture.worktree_root.to_string_lossy(),
        })),
        ActivityEntry::new(&task, ActivityKind::RunStarted, Actor::System, "mars", "claude started")
            .in_run(&run.id),
        ActivityEntry::new(
            &task,
            ActivityKind::Message,
            Actor::Agent,
            "claude",
            "I will add the guard",
        )
        .in_run(&run.id)
        .with_payload(json!({ "text": "I will add the guard and a test for it." })),
        ActivityEntry::new(
            &task,
            ActivityKind::PermissionRequest,
            Actor::Agent,
            "claude",
            "run `npm test`",
        )
        .in_run(&run.id)
        .with_payload(json!({ "requestId": "request-1", "classification": "mutating" })),
        ActivityEntry::new(
            &task,
            ActivityKind::PermissionDecision,
            Actor::Human,
            "fayeed",
            "allowed for this task",
        )
        .in_run(&run.id)
        .with_payload(json!({
            "requestId": "request-1",
            "decision": "allow_task",
            "decidedBy": "fayeed",
        })),
        ActivityEntry::new(
            &task,
            ActivityKind::FileEdit,
            Actor::Agent,
            "claude",
            "edited src/math.ts",
        )
        .in_run(&run.id)
        .with_payload(json!({
            "path": "src/math.ts",
            "change": "modified",
            "added": 3,
            "removed": 0,
            "detail": "@@ -1,3 +1,6 @@\n+if (b === 0) throw new Error('divide by zero');\n",
        })),
        ActivityEntry::new(&task, ActivityKind::Command, Actor::Agent, "claude", "npm test")
            .in_run(&run.id)
            .with_payload(json!({
                "command": "npm test",
                "status": "completed",
                "exitCode": 0,
                "output": "2 passing",
            })),
        ActivityEntry::new(
            &task,
            ActivityKind::PermissionRequest,
            Actor::Agent,
            "claude",
            "push to origin",
        )
        .in_run(&run.id)
        .with_payload(json!({ "requestId": "request-2", "classification": "mutating" })),
    ] {
        record.append(entry).expect("event should append");
    }

    (fixture, task)
}

#[test]
fn the_document_carries_transcript_commands_diffs_and_decisions() {
    let (fixture, task) = recorded();
    let document = fixture
        .record()
        .audit_markdown(&task, WHEN)
        .expect("audit should render");

    // Transcript: both sides of the conversation, in their own words.
    assert!(document.contains("Guard the divisor in divide()."));
    assert!(document.contains("I will add the guard and a test for it."));

    // Commands: what was run, how it ended, and what it printed.
    assert!(document.contains("npm test"));
    assert!(document.contains("exit 0"));
    assert!(document.contains("2 passing"));

    // Diffs: the hunk itself, not a count of hunks.
    assert!(document.contains("src/math.ts"));
    assert!(document.contains("+if (b === 0) throw new Error('divide by zero');"));
    assert!(document.contains("(+3 −0)"));

    // Decisions: who allowed what, and when.
    assert!(document.contains("request-1"));
    assert!(document.contains("allow_task"));
    assert!(document.contains("fayeed"));

    // And the branch the work happened on.
    assert!(document.contains("mars/guard-divisor"));
    assert!(document.contains("abc123def456"));
}

#[test]
fn a_request_nobody_answered_is_named_rather_than_omitted() {
    // The most important line in an audit is often the one about the decision
    // that was never made.
    let (fixture, task) = recorded();
    let document = fixture
        .record()
        .audit_markdown(&task, WHEN)
        .expect("audit should render");

    assert!(document.contains("request-2"));
    assert!(document.contains("never answered"));
}

#[test]
fn the_same_log_always_produces_the_same_document() {
    // An audit that differs between two exports of unchanged evidence cannot be
    // compared against itself, so the timestamp is an argument and not a clock.
    let (fixture, task) = recorded();
    let first = fixture.record().audit_markdown(&task, WHEN).expect("first render");
    let second = fixture.record().audit_markdown(&task, WHEN).expect("second render");
    assert_eq!(first, second);
}

#[test]
fn an_event_kind_with_no_special_rendering_still_gets_a_line() {
    let fixture = ActivityFixture::new();
    let record = fixture.record();
    let task = fixture.task("task-unknown-kind", "Odd one out");
    record
        .append(ActivityEntry::new(
            &task,
            ActivityKind::Thought,
            Actor::Agent,
            "claude",
            "considering the edge cases",
        ))
        .expect("event should append");

    let document = record.audit_markdown(&task, WHEN).expect("audit should render");
    assert!(document.contains("considering the edge cases"));
    assert!(document.contains("thought"));
}

#[test]
fn a_spilled_payload_is_resolved_into_the_document() {
    // A test log that has to be fetched from somewhere else to be read is a
    // pointer to evidence, not evidence.
    let fixture = ActivityFixture::new();
    let record = fixture.record();
    let task = fixture.task("task-spilled", "Big output");
    let marker = "the-line-that-matters";
    record
        .append(
            ActivityEntry::new(&task, ActivityKind::Command, Actor::Agent, "claude", "cargo test")
                .with_payload(json!({
                    "command": "cargo test",
                    "status": "completed",
                    "exitCode": 1,
                    "output": format!("{}\n{marker}\n", "noise\n".repeat(6_000)),
                })),
        )
        .expect("event should append");

    let document = record.audit_markdown(&task, WHEN).expect("audit should render");
    assert!(document.contains(marker));
    // A spilled row keeps only its digest, so the command and its exit code have
    // to be read back out of the blob — otherwise the audit would report the run
    // whose evidence matters most as having no command and no result.
    assert!(document.contains("cargo test"));
    assert!(document.contains("exit 1"));
}

#[test]
fn a_truncated_payload_says_so_on_the_page() {
    let fixture = ActivityFixture::new();
    let record = fixture.record();
    let task = fixture.task("task-truncated", "Very big output");
    record
        .append(
            ActivityEntry::new(&task, ActivityKind::Command, Actor::Agent, "claude", "cargo test")
                .with_payload(json!({
                    "command": "cargo test",
                    "status": "completed",
                    "output": "x".repeat(mars_activity::AUDIT_MAX_PAYLOAD_BYTES + 4_096),
                })),
        )
        .expect("event should append");

    let document = record.audit_markdown(&task, WHEN).expect("audit should render");
    // Silent truncation in an audit is a lie of omission.
    assert!(document.contains("Cut after"));
}

#[test]
fn an_empty_log_is_a_document_that_says_it_is_empty() {
    let fixture = ActivityFixture::new();
    let task = fixture.task("task-empty-log", "Nothing yet");
    let document = fixture
        .record()
        .audit_markdown(&task, WHEN)
        .expect("audit should render");

    assert!(document.contains("The log for this task is empty."));
    assert!(document.contains("No permission was ever requested"));
    assert!(document.contains("no branch was ever provisioned"));
}
