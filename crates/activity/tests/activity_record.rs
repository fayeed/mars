mod support;

use std::time::Duration;

use mars_activity::{
    ActivityEntry, ActivityKind, Actor, Attachment, NewRun, ReplayedRunState, RunRole, replay,
};
use serde_json::{Value, json};

use support::ActivityFixture;

#[test]
fn sequences_are_monotonic_per_task_and_never_interleave() {
    let fixture = ActivityFixture::new();
    let record = fixture.record();
    let first = fixture.task("task-first", "First");
    let second = fixture.task("task-second", "Second");

    for index in 0..3 {
        record
            .append(ActivityEntry::new(
                &first,
                ActivityKind::Message,
                Actor::Agent,
                "claude",
                format!("first message {index}"),
            ))
            .expect("first task event should append");
        record
            .append(ActivityEntry::new(
                &second,
                ActivityKind::Message,
                Actor::Agent,
                "codex",
                format!("second message {index}"),
            ))
            .expect("second task event should append");
    }

    let first_events = record.read(&first, 0, 100).expect("first log should read");
    let second_events = record
        .read(&second, 0, 100)
        .expect("second log should read");

    assert_eq!(
        first_events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(
        second_events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(
        record.last_sequence(&first).expect("sequence should read"),
        3
    );
    assert!(
        first_events
            .iter()
            .all(|event| event.actor_label == "claude")
    );
}

#[test]
fn written_events_can_never_be_changed_or_removed_on_their_own() {
    let fixture = ActivityFixture::new();
    let record = fixture.record();
    let task = fixture.task("task-immutable", "Immutable");
    record
        .append(ActivityEntry::new(
            &task,
            ActivityKind::Message,
            Actor::Agent,
            "claude",
            "the only account of what happened",
        ))
        .expect("event should append");

    let update = fixture.storage.with_connection(|connection| {
        connection.execute(
            "UPDATE activity_events SET summary = 'rewritten' WHERE task_id = ?1",
            [&task],
        )
    });
    let delete = fixture.storage.with_connection(|connection| {
        connection.execute("DELETE FROM activity_events WHERE task_id = ?1", [&task])
    });

    assert!(update.is_err(), "an event must not be updatable");
    assert!(delete.is_err(), "an event must not be deletable on its own");
    assert_eq!(
        record.read(&task, 0, 10).expect("log should read")[0].summary,
        "the only account of what happened"
    );

    // Deleting the task takes its log with it; that is the one permitted path.
    fixture
        .storage
        .with_connection(|connection| {
            connection.execute("DELETE FROM tasks WHERE id = ?1", [&task])
        })
        .expect("deleting a task should cascade to its log");
    assert!(
        record
            .read(&task, 0, 10)
            .expect("log should read")
            .is_empty()
    );
}

#[test]
fn large_payloads_are_content_addressed_and_stored_once() {
    let fixture = ActivityFixture::new();
    let record = fixture.record();
    let task = fixture.task("task-payloads", "Payloads");
    let output = "compiling mars-activity\n".repeat(2_000);

    let first = record
        .append(
            ActivityEntry::new(
                &task,
                ActivityKind::Command,
                Actor::Agent,
                "claude",
                "cargo build",
            )
            .with_payload(json!({ "command": "cargo build", "status": "completed" }))
            .with_attachment(Attachment::text("text/plain", output.clone()))
            .lasting(Duration::from_millis(1_250)),
        )
        .expect("first command event should append");
    let second = record
        .append(
            ActivityEntry::new(
                &task,
                ActivityKind::Command,
                Actor::Agent,
                "claude",
                "cargo build",
            )
            .with_payload(json!({ "command": "cargo build", "status": "completed" }))
            .with_attachment(Attachment::text("text/plain", output.clone())),
        )
        .expect("second command event should append");

    let digest = first
        .payload_digest
        .clone()
        .expect("a large payload should be addressed by digest");
    assert_eq!(second.payload_digest.as_deref(), Some(digest.as_str()));
    assert_eq!(first.duration_ms, Some(1_250));

    let stored_rows: i64 = fixture
        .storage
        .with_connection(|connection| {
            connection.query_row("SELECT COUNT(*) FROM activity_payloads", [], |row| {
                row.get(0)
            })
        })
        .expect("payload count should read");
    assert_eq!(stored_rows, 1, "identical payloads should be stored once");

    let bytes = record
        .payload_bytes(&digest)
        .expect("payload should resolve by digest");
    assert_eq!(String::from_utf8_lossy(&bytes), output);
    assert_eq!(
        first.payload["attachment"]["byteSize"]
            .as_u64()
            .expect("attachment should record its size"),
        output.len() as u64
    );
}

#[test]
fn oversized_inline_payloads_spill_to_the_payload_store() {
    let fixture = ActivityFixture::new();
    let record = fixture.record();
    let task = fixture.task("task-spill", "Spill");

    let event = record
        .append(
            ActivityEntry::new(
                &task,
                ActivityKind::ToolCall,
                Actor::Agent,
                "codex",
                "read a very large file",
            )
            .with_payload(json!({ "contents": "x".repeat(32 * 1024) })),
        )
        .expect("large inline payload should append");

    assert_eq!(event.payload["spilled"], Value::Bool(true));
    let digest = event
        .payload_digest
        .clone()
        .expect("spilled payload should carry a digest");
    let bytes = record
        .payload_bytes(&digest)
        .expect("spilled payload should resolve");
    let resolved: Value = serde_json::from_slice(&bytes).expect("spilled payload should be JSON");
    assert_eq!(resolved["contents"].as_str().map(str::len), Some(32 * 1024));
}

#[test]
fn replay_reconstructs_the_task_from_the_log_alone() {
    let fixture = ActivityFixture::new();
    let record = fixture.record();
    let runs = fixture.runs();
    let task = fixture.task("task-replay", "Replay");
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

    let events = [
        ActivityEntry::new(
            &task,
            ActivityKind::WorktreeCreated,
            Actor::System,
            "mars",
            "worktree ready",
        )
        .with_payload(json!({
            "branchName": "mars/replay",
            "baseRef": "main",
            "baseOid": "abc123",
            "path": fixture.worktree_root.to_string_lossy(),
        })),
        ActivityEntry::new(
            &task,
            ActivityKind::RunStarted,
            Actor::System,
            "mars",
            "claude started",
        )
        .in_run(&run.id),
        ActivityEntry::new(
            &task,
            ActivityKind::Message,
            Actor::Agent,
            "claude",
            "I will add the guard",
        )
        .in_run(&run.id),
        ActivityEntry::new(
            &task,
            ActivityKind::FileEdit,
            Actor::Agent,
            "claude",
            "edited src/math.ts",
        )
        .in_run(&run.id)
        .with_payload(json!({ "path": "src/math.ts", "change": "modified" })),
        ActivityEntry::new(
            &task,
            ActivityKind::FileEdit,
            Actor::Agent,
            "claude",
            "edited src/math.ts again",
        )
        .in_run(&run.id)
        .with_payload(json!({ "path": "src/math.ts", "change": "modified" })),
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
        .with_payload(json!({ "requestId": "request-1", "decision": "allow_task" })),
        ActivityEntry::new(
            &task,
            ActivityKind::Command,
            Actor::Agent,
            "claude",
            "npm test",
        )
        .in_run(&run.id)
        .with_payload(json!({ "command": "npm test", "status": "completed", "exitCode": 0 }))
        .lasting(Duration::from_millis(4_100)),
        ActivityEntry::new(
            &task,
            ActivityKind::TestRun,
            Actor::Agent,
            "claude",
            "12 passed",
        )
        .in_run(&run.id)
        .with_payload(json!({ "command": "npm test", "status": "passed" })),
        ActivityEntry::new(
            &task,
            ActivityKind::Done,
            Actor::Agent,
            "claude",
            "finished",
        )
        .in_run(&run.id)
        .with_payload(json!({ "success": true })),
    ];
    for entry in events {
        record.append(entry).expect("event should append");
    }

    let projection = replay(&record, &task).expect("log should replay");

    assert_eq!(projection.event_count, 10);
    assert_eq!(projection.last_sequence, 10);
    assert_eq!(projection.runs.len(), 1);
    assert_eq!(projection.runs[0].state, ReplayedRunState::Completed);
    assert_eq!(projection.runs[0].message_count, 1);
    assert!(projection.runs[0].blocked_on_request.is_none());
    assert!(projection.pending_permissions.is_empty());
    assert_eq!(projection.decided_permissions.len(), 1);
    assert_eq!(projection.decided_permissions[0].decision, "allow_task");
    assert_eq!(projection.decided_permissions[0].decided_by, "fayeed");
    assert_eq!(projection.files_touched.len(), 1);
    assert_eq!(projection.files_touched[0].edits, 2);
    assert_eq!(projection.commands.len(), 1);
    assert_eq!(projection.commands[0].exit_code, Some(0));
    assert_eq!(projection.commands[0].duration_ms, Some(4_100));
    assert_eq!(projection.tests.len(), 1);
    assert_eq!(
        projection
            .worktree
            .as_ref()
            .and_then(|worktree| worktree.branch_name.as_deref()),
        Some("mars/replay")
    );
}

#[test]
fn an_undecided_permission_request_leaves_the_run_blocked() {
    let fixture = ActivityFixture::new();
    let record = fixture.record();
    let runs = fixture.runs();
    let task = fixture.task("task-blocked", "Blocked");
    let run = runs
        .start(NewRun {
            task_id: task.clone(),
            agent_id: "codex".into(),
            adapter_kind: "codex".into(),
            role: RunRole::Implementer,
            capabilities: json!({ "writeFiles": true }),
            context_pack_digest: None,
        })
        .expect("run should start");

    record
        .append(
            ActivityEntry::new(
                &task,
                ActivityKind::RunStarted,
                Actor::System,
                "mars",
                "codex started",
            )
            .in_run(&run.id),
        )
        .expect("start should append");
    record
        .append(
            ActivityEntry::new(
                &task,
                ActivityKind::PermissionRequest,
                Actor::Agent,
                "codex",
                "delete build output",
            )
            .in_run(&run.id)
            .with_payload(json!({ "requestId": "request-9", "classification": "destructive" })),
        )
        .expect("request should append");

    let projection = replay(&record, &task).expect("log should replay");

    assert_eq!(projection.runs[0].state, ReplayedRunState::Blocked);
    assert_eq!(
        projection.runs[0].blocked_on_request.as_deref(),
        Some("request-9")
    );
    assert_eq!(projection.pending_permissions.len(), 1);
    assert_eq!(
        projection.pending_permissions[0].classification.as_deref(),
        Some("destructive")
    );
}

#[test]
fn export_is_newline_delimited_and_carries_its_payloads() {
    let fixture = ActivityFixture::new();
    let record = fixture.record();
    let task = fixture.task("task-export", "Export");
    let diff = "diff --git a/src/math.ts b/src/math.ts\n".repeat(500);

    record
        .append(ActivityEntry::new(
            &task,
            ActivityKind::Message,
            Actor::Agent,
            "claude",
            "starting",
        ))
        .expect("message should append");
    record
        .append(
            ActivityEntry::new(
                &task,
                ActivityKind::FileEdit,
                Actor::Agent,
                "claude",
                "wrote the diff",
            )
            .with_payload(json!({ "path": "src/math.ts", "change": "modified" }))
            .with_attachment(Attachment::text("text/x-diff", diff.clone())),
        )
        .expect("file edit should append");

    let exported = record.export(&task).expect("log should export");
    let lines = exported.lines().collect::<Vec<_>>();

    assert_eq!(lines.len(), 2);
    let second: Value = serde_json::from_str(lines[1]).expect("exported line should be JSON");
    assert_eq!(second["sequence"], json!(2));
    assert_eq!(second["kind"], json!("file_edit"));
    assert_eq!(
        second["resolvedPayload"]
            .as_str()
            .expect("an exported attachment should resolve to its bytes"),
        diff
    );
}
