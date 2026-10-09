//! Replaying the activity record into the state the UI shows.
//!
//! Nothing here reads a status column. A projection is produced only from
//! events, which is what makes reload, crash recovery, and export agree with
//! each other: they are the same fold over the same log.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ActivityError;
use crate::record::{ActivityEvent, ActivityKind, ActivityRecord};

/// What the log says happened to one run.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayedRunState {
    Running,
    /// The last thing the run did was ask permission, and no decision followed.
    Blocked,
    Completed,
    Failed,
    /// Someone stopped it before it finished.
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayedRun {
    pub run_id: String,
    pub actor_label: String,
    pub state: ReplayedRunState,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub first_sequence: i64,
    pub last_sequence: i64,
    pub message_count: usize,
    pub blocked_on_request: Option<String>,
    pub failure: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingPermission {
    pub request_id: String,
    pub run_id: Option<String>,
    pub summary: String,
    pub classification: Option<String>,
    pub requested_at: String,
    pub sequence: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecidedPermission {
    pub request_id: String,
    pub decision: String,
    pub decided_by: String,
    pub decided_at: String,
    pub sequence: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileTouch {
    pub path: String,
    pub change: String,
    pub run_id: Option<String>,
    pub last_sequence: i64,
    pub edits: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandRun {
    pub command: String,
    pub status: String,
    /// The provider's id for the call, which its completion carries too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    pub exit_code: Option<i64>,
    pub duration_ms: Option<i64>,
    pub output_digest: Option<String>,
    pub run_id: Option<String>,
    pub sequence: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestOutcome {
    pub command: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    pub duration_ms: Option<i64>,
    pub result_digest: Option<String>,
    pub sequence: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeView {
    pub branch_name: Option<String>,
    pub base_ref: Option<String>,
    pub base_oid: Option<String>,
    pub path: Option<String>,
    pub state: String,
    pub sequence: i64,
}

/// The whole of a task's state, folded from its log.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskProjection {
    pub task_id: String,
    pub last_sequence: i64,
    pub event_count: usize,
    pub runs: Vec<ReplayedRun>,
    pub pending_permissions: Vec<PendingPermission>,
    pub decided_permissions: Vec<DecidedPermission>,
    pub files_touched: Vec<FileTouch>,
    pub commands: Vec<CommandRun>,
    pub tests: Vec<TestOutcome>,
    pub errors: Vec<String>,
    pub worktree: Option<WorktreeView>,
}

/// Read a task's whole log and fold it.
pub fn replay(record: &ActivityRecord, task_id: &str) -> Result<TaskProjection, ActivityError> {
    let mut events = Vec::new();
    let mut after = 0;
    loop {
        let page = record.read(task_id, after, 1_000)?;
        if page.is_empty() {
            break;
        }
        after = page[page.len() - 1].sequence;
        events.extend(page);
    }
    Ok(project(task_id, &events))
}

/// Fold an already-loaded slice of the log. Separated from [`replay`] so the
/// same rules apply to a live stream and to a recovered one.
#[must_use]
pub fn project(task_id: &str, events: &[ActivityEvent]) -> TaskProjection {
    let mut runs: BTreeMap<String, ReplayedRun> = BTreeMap::new();
    let mut run_order: Vec<String> = Vec::new();
    let mut pending: BTreeMap<String, PendingPermission> = BTreeMap::new();
    let mut decided: Vec<DecidedPermission> = Vec::new();
    let mut files: BTreeMap<String, FileTouch> = BTreeMap::new();
    let mut commands: Vec<CommandRun> = Vec::new();
    let mut tests: Vec<TestOutcome> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut worktree: Option<WorktreeView> = None;
    let mut last_sequence = 0;

    for event in events {
        last_sequence = last_sequence.max(event.sequence);

        if let Some(run_id) = event.run_id.as_deref() {
            let entry = runs.entry(run_id.to_owned()).or_insert_with(|| {
                run_order.push(run_id.to_owned());
                ReplayedRun {
                    run_id: run_id.to_owned(),
                    actor_label: event.actor_label.clone(),
                    state: ReplayedRunState::Running,
                    started_at: event.occurred_at.clone(),
                    ended_at: None,
                    first_sequence: event.sequence,
                    last_sequence: event.sequence,
                    message_count: 0,
                    blocked_on_request: None,
                    failure: None,
                }
            });
            entry.last_sequence = event.sequence;
            if matches!(event.kind, ActivityKind::Message) {
                entry.message_count += 1;
            }
        }

        match event.kind {
            ActivityKind::PermissionRequest => {
                if let Some(request_id) = string_field(&event.payload, "requestId") {
                    pending.insert(
                        request_id.clone(),
                        PendingPermission {
                            request_id: request_id.clone(),
                            run_id: event.run_id.clone(),
                            summary: event.summary.clone(),
                            classification: string_field(&event.payload, "classification"),
                            requested_at: event.occurred_at.clone(),
                            sequence: event.sequence,
                        },
                    );
                    if let Some(run_id) = event.run_id.as_deref()
                        && let Some(run) = runs.get_mut(run_id)
                    {
                        run.state = ReplayedRunState::Blocked;
                        run.blocked_on_request = Some(request_id);
                    }
                }
            }
            ActivityKind::PermissionDecision => {
                if let Some(request_id) = string_field(&event.payload, "requestId") {
                    pending.remove(&request_id);
                    decided.push(DecidedPermission {
                        request_id: request_id.clone(),
                        decision: string_field(&event.payload, "decision")
                            .unwrap_or_else(|| "unknown".into()),
                        decided_by: event.actor_label.clone(),
                        decided_at: event.occurred_at.clone(),
                        sequence: event.sequence,
                    });
                    for run in runs.values_mut() {
                        if run.blocked_on_request.as_deref() == Some(request_id.as_str()) {
                            run.blocked_on_request = None;
                            if run.state == ReplayedRunState::Blocked {
                                run.state = ReplayedRunState::Running;
                            }
                        }
                    }
                }
            }
            ActivityKind::FileEdit => {
                if let Some(path) = string_field(&event.payload, "path") {
                    let entry = files.entry(path.clone()).or_insert_with(|| FileTouch {
                        path,
                        change: string_field(&event.payload, "change")
                            .unwrap_or_else(|| "modified".into()),
                        run_id: event.run_id.clone(),
                        last_sequence: event.sequence,
                        edits: 0,
                    });
                    entry.edits += 1;
                    entry.last_sequence = event.sequence;
                    if let Some(change) = string_field(&event.payload, "change") {
                        entry.change = change;
                    }
                }
            }
            ActivityKind::Command => {
                let call_id = string_field(&event.payload, "callId");
                let status =
                    string_field(&event.payload, "status").unwrap_or_else(|| "unknown".into());
                let exit_code = event.payload.get("exitCode").and_then(Value::as_i64);
                // A completion names the call it finishes; it updates that
                // command rather than reading as a second one.
                let started = call_id.as_deref().and_then(|call_id| {
                    commands
                        .iter_mut()
                        .rev()
                        .find(|command| command.call_id.as_deref() == Some(call_id))
                });
                if let Some(command) = started {
                    command.status = status;
                    command.exit_code = exit_code.or(command.exit_code);
                    command.duration_ms = event.duration_ms.or(command.duration_ms);
                    command.output_digest = event
                        .payload_digest
                        .clone()
                        .or(command.output_digest.take());
                    command.sequence = event.sequence;
                } else {
                    commands.push(CommandRun {
                        command: string_field(&event.payload, "command")
                            .unwrap_or_else(|| event.summary.clone()),
                        status,
                        call_id,
                        exit_code,
                        duration_ms: event.duration_ms,
                        output_digest: event.payload_digest.clone(),
                        run_id: event.run_id.clone(),
                        sequence: event.sequence,
                    });
                }
            }
            ActivityKind::TestRun => {
                let call_id = string_field(&event.payload, "callId");
                let status =
                    string_field(&event.payload, "status").unwrap_or_else(|| "unknown".into());
                let exit_code = event.payload.get("exitCode").and_then(Value::as_i64);
                let started = call_id.as_deref().and_then(|call_id| {
                    tests
                        .iter_mut()
                        .rev()
                        .find(|test| test.call_id.as_deref() == Some(call_id))
                });
                if let Some(test) = started {
                    test.status = status;
                    test.exit_code = exit_code.or(test.exit_code);
                    test.duration_ms = event.duration_ms.or(test.duration_ms);
                    test.result_digest = event.payload_digest.clone().or(test.result_digest.take());
                    test.sequence = event.sequence;
                } else {
                    tests.push(TestOutcome {
                        command: string_field(&event.payload, "command")
                            .unwrap_or_else(|| event.summary.clone()),
                        status,
                        call_id,
                        exit_code,
                        duration_ms: event.duration_ms,
                        result_digest: event.payload_digest.clone(),
                        sequence: event.sequence,
                    });
                }
            }
            // A provider frame Mars could not read is kept for diagnosis. It
            // says nothing about whether the run went well.
            ActivityKind::Error
                if event.payload.get("diagnostic").and_then(Value::as_bool) == Some(true) => {}
            ActivityKind::Error => {
                errors.push(event.summary.clone());
                if let Some(run_id) = event.run_id.as_deref()
                    && let Some(run) = runs.get_mut(run_id)
                {
                    run.failure = Some(event.summary.clone());
                }
            }
            ActivityKind::Done => {
                if let Some(run_id) = event.run_id.as_deref()
                    && let Some(run) = runs.get_mut(run_id)
                {
                    let success = event
                        .payload
                        .get("success")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    let stopped = event
                        .payload
                        .get("stopped")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    run.state = if success {
                        ReplayedRunState::Completed
                    } else if stopped {
                        ReplayedRunState::Cancelled
                    } else {
                        ReplayedRunState::Failed
                    };
                    run.ended_at = Some(event.occurred_at.clone());
                    run.blocked_on_request = None;
                    if !success && !stopped && run.failure.is_none() {
                        run.failure = Some(event.summary.clone());
                    }
                }
            }
            ActivityKind::WorktreePlanned
            | ActivityKind::WorktreeCreated
            | ActivityKind::WorktreeRemoved
            | ActivityKind::WorktreeRestored => {
                worktree = Some(WorktreeView {
                    branch_name: string_field(&event.payload, "branchName"),
                    base_ref: string_field(&event.payload, "baseRef"),
                    base_oid: string_field(&event.payload, "baseOid"),
                    path: string_field(&event.payload, "path"),
                    state: match event.kind {
                        ActivityKind::WorktreePlanned => "planned",
                        ActivityKind::WorktreeCreated => "ready",
                        ActivityKind::WorktreeRemoved => "removed",
                        _ => "restored",
                    }
                    .to_owned(),
                    sequence: event.sequence,
                });
            }
            ActivityKind::Message
            | ActivityKind::Thought
            | ActivityKind::ToolCall
            | ActivityKind::RunStarted
            | ActivityKind::RecoveryNoted => {}
        }
    }

    let ordered_runs = run_order
        .into_iter()
        .filter_map(|run_id| runs.remove(&run_id))
        .collect();

    TaskProjection {
        task_id: task_id.to_owned(),
        last_sequence,
        event_count: events.len(),
        runs: ordered_runs,
        pending_permissions: pending.into_values().collect(),
        decided_permissions: decided,
        files_touched: files.into_values().collect(),
        commands,
        tests,
        errors,
        worktree,
    }
}

fn string_field(payload: &Value, field: &str) -> Option<String> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}
