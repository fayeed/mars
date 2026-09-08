//! Crash recovery.
//!
//! On start, Mars looks for runs that were not terminal when it stopped. It
//! never resumes one. It reconstructs what the log says was in flight, what was
//! written to disk, and what it cannot verify, and offers the human choices that
//! each spell out their consequence.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mars_storage::Storage;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::ActivityError;
use crate::record::{ActivityEntry, ActivityKind, ActivityRecord, Actor};
use crate::replay::{TaskProjection, replay};
use crate::runs::{RunRecord, RunStatus, RunStore};

/// Something the run had started and never reported the end of.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InFlightItem {
    pub kind: String,
    pub summary: String,
    pub sequence: i64,
    pub started_at: String,
}

/// A file the log says the agent wrote, and whether it is still on disk.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WrittenFile {
    pub path: String,
    pub change: String,
    pub edits: usize,
    pub last_sequence: i64,
    /// `None` when the worktree path is unknown, so presence could not be checked.
    pub present_on_disk: Option<bool>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    /// Start a fresh run against the same worktree, telling the new agent what
    /// the old one had already done.
    Resume,
    /// Keep the worktree, mark the run abandoned, change nothing on disk.
    Discard,
    /// Reset the worktree to the run's starting point and run the task again.
    Rerun,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryOption {
    pub action: RecoveryAction,
    pub label: String,
    pub consequence: String,
}

/// Everything a human needs to decide what to do with an interrupted run.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InterruptedRun {
    pub run_id: String,
    pub task_id: String,
    pub agent_id: String,
    pub adapter_kind: String,
    pub role: String,
    pub status_when_stopped: String,
    pub started_at: String,
    pub last_event_at: Option<String>,
    pub last_sequence: i64,
    pub in_flight: Vec<InFlightItem>,
    pub files_written: Vec<WrittenFile>,
    pub unverified: Vec<String>,
    pub options: Vec<RecoveryOption>,
}

#[derive(Clone)]
pub struct RecoveryService {
    record: ActivityRecord,
    runs: RunStore,
}

impl RecoveryService {
    #[must_use]
    pub fn new(storage: Arc<Storage>, payload_root: impl AsRef<Path>) -> Self {
        Self {
            record: ActivityRecord::new(storage.clone(), payload_root),
            runs: RunStore::new(storage),
        }
    }

    #[must_use]
    pub fn from_parts(record: ActivityRecord, runs: RunStore) -> Self {
        Self { record, runs }
    }

    /// Find every run that was mid-flight when Mars stopped. Read-only: it
    /// changes no status and starts no process.
    pub fn detect(&self) -> Result<Vec<InterruptedRun>, ActivityError> {
        let mut interrupted = Vec::new();
        for run in self.runs.unfinished()? {
            let projection = replay(&self.record, &run.task_id)?;
            interrupted.push(describe(&run, &projection));
        }
        Ok(interrupted)
    }

    /// Record that a run was found interrupted. This is the only status change
    /// recovery makes on its own, and it moves the run further from running,
    /// never closer.
    pub fn mark_interrupted(&self, run_id: &str) -> Result<InterruptedRun, ActivityError> {
        let run = self.runs.get(run_id)?;
        if run.status.is_terminal() {
            return Err(ActivityError::RunAlreadyFinished(run_id.to_owned()));
        }
        let projection = replay(&self.record, &run.task_id)?;
        let described = describe(&run, &projection);
        self.record.append(
            ActivityEntry::new(
                &run.task_id,
                ActivityKind::RecoveryNoted,
                Actor::System,
                "mars",
                format!(
                    "Run {run_id} was interrupted; {} item(s) in flight, {} file(s) written",
                    described.in_flight.len(),
                    described.files_written.len()
                ),
            )
            .in_run(run_id)
            .with_payload(serde_json::to_value(&described)?),
        )?;
        self.runs.set_status(run_id, RunStatus::Interrupted)?;
        Ok(described)
    }

    /// Record the human's choice. The choice itself is carried out by the
    /// caller; this makes it part of the transcript first.
    pub fn record_choice(
        &self,
        run_id: &str,
        action: RecoveryAction,
        decided_by: &str,
    ) -> Result<(), ActivityError> {
        let run = self.runs.get(run_id)?;
        let action_name = match action {
            RecoveryAction::Resume => "resume",
            RecoveryAction::Discard => "discard",
            RecoveryAction::Rerun => "rerun",
        };
        self.record.append(
            ActivityEntry::new(
                &run.task_id,
                ActivityKind::RecoveryNoted,
                Actor::Human,
                decided_by,
                format!("Chose to {action_name} interrupted run {run_id}"),
            )
            .in_run(run_id)
            .with_payload(json!({
                "runId": run_id,
                "action": action_name,
                "decidedBy": decided_by,
            })),
        )?;
        Ok(())
    }
}

fn describe(run: &RunRecord, projection: &TaskProjection) -> InterruptedRun {
    let replayed = projection
        .runs
        .iter()
        .find(|candidate| candidate.run_id == run.id);

    let mut in_flight = Vec::new();
    for command in projection
        .commands
        .iter()
        .filter(|command| command.run_id.as_deref() == Some(run.id.as_str()))
    {
        if command.status != "completed" && command.status != "failed" {
            in_flight.push(InFlightItem {
                kind: "command".into(),
                summary: command.command.clone(),
                sequence: command.sequence,
                started_at: String::new(),
            });
        }
    }
    for pending in projection
        .pending_permissions
        .iter()
        .filter(|pending| pending.run_id.as_deref() == Some(run.id.as_str()))
    {
        in_flight.push(InFlightItem {
            kind: "permission_request".into(),
            summary: pending.summary.clone(),
            sequence: pending.sequence,
            started_at: pending.requested_at.clone(),
        });
    }
    in_flight.sort_by_key(|item| item.sequence);

    let worktree_root = projection
        .worktree
        .as_ref()
        .and_then(|worktree| worktree.path.as_deref())
        .map(PathBuf::from);

    let files_written = projection
        .files_touched
        .iter()
        .filter(|file| file.run_id.as_deref() == Some(run.id.as_str()))
        .map(|file| WrittenFile {
            path: file.path.clone(),
            change: file.change.clone(),
            edits: file.edits,
            last_sequence: file.last_sequence,
            present_on_disk: worktree_root
                .as_ref()
                .map(|root| root.join(&file.path).exists()),
        })
        .collect::<Vec<_>>();

    let mut unverified = Vec::new();
    if !in_flight.is_empty() {
        unverified.push(format!(
            "{} operation(s) started and never reported an outcome",
            in_flight.len()
        ));
    }
    if !files_written.is_empty() {
        unverified.push(format!(
            "{} file(s) were written after the last recorded test run",
            files_written
                .iter()
                .filter(|file| projection
                    .tests
                    .last()
                    .is_none_or(|test| test.sequence < file.last_sequence))
                .count()
        ));
    }
    if projection.tests.is_empty() {
        unverified.push("no test run was recorded for this work".into());
    }
    unverified.push(format!(
        "the run never reported completion; its last event was sequence {}",
        replayed.map_or(run.last_sequence, |replayed| replayed.last_sequence)
    ));

    InterruptedRun {
        run_id: run.id.clone(),
        task_id: run.task_id.clone(),
        agent_id: run.agent_id.clone(),
        adapter_kind: run.adapter_kind.clone(),
        role: run.role.as_str().to_owned(),
        status_when_stopped: run.status.as_str().to_owned(),
        started_at: run.started_at.clone(),
        last_event_at: None,
        last_sequence: replayed.map_or(run.last_sequence, |replayed| replayed.last_sequence),
        in_flight,
        files_written,
        unverified,
        options: recovery_options(),
    }
}

fn recovery_options() -> Vec<RecoveryOption> {
    vec![
        RecoveryOption {
            action: RecoveryAction::Resume,
            label: "Resume in the same worktree".into(),
            consequence:
                "Starts a new run against the files already on disk. Uncommitted changes from the \
                 interrupted run stay and become part of the new run's work."
                    .into(),
        },
        RecoveryOption {
            action: RecoveryAction::Discard,
            label: "Discard the run, keep the files".into(),
            consequence:
                "Marks the run abandoned and changes nothing on disk. The worktree keeps whatever \
                 the interrupted run wrote, for you to inspect or revert by hand."
                    .into(),
        },
        RecoveryOption {
            action: RecoveryAction::Rerun,
            label: "Reset the worktree and run again".into(),
            consequence:
                "Discards every uncommitted change the interrupted run made, returns the worktree \
                 to its branch tip, and starts the task from the beginning. This cannot be undone."
                    .into(),
        },
    ]
}

/// A convenience for callers that only need to know whether recovery is pending.
pub fn interrupted_payload(runs: &[InterruptedRun]) -> Value {
    json!({
        "interruptedRuns": runs.len(),
        "taskIds": runs.iter().map(|run| run.task_id.clone()).collect::<Vec<_>>(),
    })
}
