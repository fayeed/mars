mod support;

use std::fs;

use mars_activity::{
    ActivityEntry, ActivityKind, Actor, NewRun, RecoveryAction, RecoveryService, RunRole,
    RunStatus, replay,
};
use serde_json::json;

use support::ActivityFixture;

/// A run left mid-flight, with one file written, one command never finished, and
/// one permission request never answered.
fn interrupted_fixture() -> (ActivityFixture, String, String) {
    let fixture = ActivityFixture::new();
    let record = fixture.record();
    let runs = fixture.runs();
    let task = fixture.task("task-crash", "Crash");
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
    runs.set_status(&run.id, RunStatus::Running)
        .expect("run should be running");

    fs::write(
        fixture.worktree_root.join("touched.ts"),
        "export const a = 1;\n",
    )
    .expect("worktree file should write");

    let entries = [
        ActivityEntry::new(
            &task,
            ActivityKind::WorktreeCreated,
            Actor::System,
            "mars",
            "worktree ready",
        )
        .with_payload(json!({
            "branchName": "mars/crash",
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
            ActivityKind::FileEdit,
            Actor::Agent,
            "claude",
            "wrote touched.ts",
        )
        .in_run(&run.id)
        .with_payload(json!({ "path": "touched.ts", "change": "created" })),
        ActivityEntry::new(
            &task,
            ActivityKind::FileEdit,
            Actor::Agent,
            "claude",
            "wrote missing.ts",
        )
        .in_run(&run.id)
        .with_payload(json!({ "path": "missing.ts", "change": "created" })),
        ActivityEntry::new(
            &task,
            ActivityKind::Command,
            Actor::Agent,
            "claude",
            "npm run build",
        )
        .in_run(&run.id)
        .with_payload(json!({ "command": "npm run build", "status": "started" })),
        ActivityEntry::new(
            &task,
            ActivityKind::PermissionRequest,
            Actor::Agent,
            "claude",
            "write outside the declared scope",
        )
        .in_run(&run.id)
        .with_payload(json!({ "requestId": "request-crash", "classification": "mutating" })),
    ];
    for entry in entries {
        record.append(entry).expect("event should append");
    }

    (fixture, task, run.id)
}

#[test]
fn detects_interrupted_runs_and_reports_what_is_unverified() {
    let (fixture, _task, run_id) = interrupted_fixture();
    let recovery = RecoveryService::new(fixture.storage.clone(), &fixture.payload_root);

    let interrupted = recovery.detect().expect("recovery should inspect runs");

    assert_eq!(interrupted.len(), 1);
    let run = &interrupted[0];
    assert_eq!(run.run_id, run_id);
    assert_eq!(run.status_when_stopped, "running");
    assert_eq!(run.adapter_kind, "claude");
    assert_eq!(run.role, "implementer");

    let in_flight_kinds = run
        .in_flight
        .iter()
        .map(|item| item.kind.as_str())
        .collect::<Vec<_>>();
    assert_eq!(in_flight_kinds, vec!["command", "permission_request"]);

    let written = run
        .files_written
        .iter()
        .map(|file| (file.path.as_str(), file.present_on_disk))
        .collect::<Vec<_>>();
    assert!(written.contains(&("touched.ts", Some(true))));
    assert!(
        written.contains(&("missing.ts", Some(false))),
        "a file the log claims was written but is absent must be reported as absent"
    );

    assert!(
        run.unverified
            .iter()
            .any(|note| note.contains("never reported an outcome"))
    );
    assert!(
        run.unverified
            .iter()
            .any(|note| note.contains("no test run was recorded"))
    );
}

#[test]
fn detection_changes_nothing_and_resumes_nothing() {
    let (fixture, task, run_id) = interrupted_fixture();
    let recovery = RecoveryService::new(fixture.storage.clone(), &fixture.payload_root);
    let record = fixture.record();
    let before = record.last_sequence(&task).expect("sequence should read");

    recovery.detect().expect("detection should succeed");
    recovery.detect().expect("detection should be repeatable");

    assert_eq!(
        record.last_sequence(&task).expect("sequence should read"),
        before,
        "detection must not write to the log"
    );
    assert_eq!(
        fixture.runs().get(&run_id).expect("run should read").status,
        RunStatus::Running,
        "detection must not change a run's status"
    );
}

#[test]
fn marking_a_run_interrupted_records_it_and_moves_it_out_of_running() {
    let (fixture, task, run_id) = interrupted_fixture();
    let recovery = RecoveryService::new(fixture.storage.clone(), &fixture.payload_root);
    let record = fixture.record();

    let described = recovery
        .mark_interrupted(&run_id)
        .expect("run should be markable");

    assert_eq!(described.run_id, run_id);
    assert_eq!(
        fixture.runs().get(&run_id).expect("run should read").status,
        RunStatus::Interrupted
    );

    let events = record.read(&task, 0, 100).expect("log should read");
    let noted = events
        .iter()
        .find(|event| event.kind == ActivityKind::RecoveryNoted)
        .expect("recovery should be recorded in the log");
    assert_eq!(noted.actor, Actor::System);
    assert!(noted.summary.contains("was interrupted"));

    assert!(
        recovery.mark_interrupted(&run_id).is_err(),
        "a finished run must not be marked interrupted twice"
    );
    assert!(
        recovery
            .detect()
            .expect("detection should succeed")
            .is_empty(),
        "an interrupted run is no longer a recovery candidate"
    );
}

#[test]
fn every_recovery_option_states_its_consequence() {
    let (fixture, _task, run_id) = interrupted_fixture();
    let recovery = RecoveryService::new(fixture.storage.clone(), &fixture.payload_root);

    let interrupted = recovery.detect().expect("detection should succeed");
    let options = &interrupted[0].options;

    assert_eq!(options.len(), 3);
    assert!(
        options
            .iter()
            .all(|option| option.consequence.len() > 40 && option.consequence.ends_with('.'))
    );
    let rerun = options
        .iter()
        .find(|option| option.action == RecoveryAction::Rerun)
        .expect("rerun should be offered");
    assert!(rerun.consequence.contains("cannot be undone"));
    assert!(
        options
            .iter()
            .find(|option| option.action == RecoveryAction::Discard)
            .expect("discard should be offered")
            .consequence
            .contains("changes nothing on disk")
    );
    assert_eq!(run_id, interrupted[0].run_id);
}

#[test]
fn the_humans_recovery_choice_becomes_part_of_the_transcript() {
    let (fixture, task, run_id) = interrupted_fixture();
    let recovery = RecoveryService::new(fixture.storage.clone(), &fixture.payload_root);
    let record = fixture.record();

    recovery
        .mark_interrupted(&run_id)
        .expect("run should be markable");
    recovery
        .record_choice(&run_id, RecoveryAction::Discard, "fayeed")
        .expect("choice should record");

    let projection = replay(&record, &task).expect("log should replay");
    let events = record.read(&task, 0, 100).expect("log should read");
    let choice = events
        .last()
        .expect("the choice should be the newest event in the log");

    assert_eq!(choice.actor, Actor::Human);
    assert_eq!(choice.actor_label, "fayeed");
    assert_eq!(choice.payload["action"], json!("discard"));
    assert!(projection.last_sequence >= choice.sequence);
}
