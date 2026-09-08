//! Agent runs: the task-level record of one agent execution.
//!
//! A run is deliberately thinner than the log it anchors. It holds the identity
//! of the agent, the role it was given, the capability ceiling it was
//! constructed with, and the digest of the context pack that was actually sent.
//! Its status is a convenience for querying; the authoritative account of what
//! happened is the activity record.

use std::sync::Arc;

use mars_storage::Storage;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::ActivityError;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Starting,
    Running,
    /// Stopped, waiting for a human decision on a permission request.
    Blocked,
    Interrupting,
    Completed,
    Failed,
    Cancelled,
    /// The process was alive when Mars stopped. Nothing auto-resumes.
    Interrupted,
}

impl RunStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Blocked => "blocked",
            Self::Interrupting => "interrupting",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }

    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }

    fn parse(value: &str) -> Result<Self, ActivityError> {
        match value {
            "starting" => Ok(Self::Starting),
            "running" => Ok(Self::Running),
            "blocked" => Ok(Self::Blocked),
            "interrupting" => Ok(Self::Interrupting),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "interrupted" => Ok(Self::Interrupted),
            other => Err(ActivityError::InvalidStoredStatus(other.to_owned())),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunRole {
    Implementer,
    Reviewer,
}

impl RunRole {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Implementer => "implementer",
            Self::Reviewer => "reviewer",
        }
    }

    fn parse(value: &str) -> Result<Self, ActivityError> {
        match value {
            "implementer" => Ok(Self::Implementer),
            "reviewer" => Ok(Self::Reviewer),
            other => Err(ActivityError::InvalidStoredRole(other.to_owned())),
        }
    }
}

#[derive(Clone, Debug)]
pub struct NewRun {
    pub task_id: String,
    pub agent_id: String,
    pub adapter_kind: String,
    pub role: RunRole,
    pub capabilities: Value,
    pub context_pack_digest: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub id: String,
    pub task_id: String,
    pub session_id: Option<String>,
    pub agent_id: String,
    pub adapter_kind: String,
    pub role: RunRole,
    pub capabilities: Value,
    pub context_pack_digest: Option<String>,
    pub status: RunStatus,
    pub last_sequence: i64,
    pub error: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
}

#[derive(Clone)]
pub struct RunStore {
    storage: Arc<Storage>,
}

impl RunStore {
    #[must_use]
    pub fn new(storage: Arc<Storage>) -> Self {
        Self { storage }
    }

    pub fn start(&self, run: NewRun) -> Result<RunRecord, ActivityError> {
        let id = Uuid::now_v7().to_string();
        let started_at = OffsetDateTime::now_utc().format(&Rfc3339)?;
        let capabilities = serde_json::to_string(&run.capabilities)?;
        self.storage.with_connection(|connection| {
            connection.execute(
                "INSERT INTO agent_runs
                     (id, task_id, session_id, agent_id, adapter_kind, role, capabilities_json,
                      context_pack_digest, status, last_sequence, error, started_at, ended_at)
                 VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, 'starting', 0, NULL, ?8, NULL)",
                params![
                    &id,
                    &run.task_id,
                    &run.agent_id,
                    &run.adapter_kind,
                    run.role.as_str(),
                    &capabilities,
                    run.context_pack_digest.as_deref(),
                    &started_at,
                ],
            )?;
            Ok(())
        })?;
        self.get(&id)
    }

    pub fn attach_session(&self, run_id: &str, session_id: &str) -> Result<(), ActivityError> {
        self.storage.with_connection(|connection| {
            connection.execute(
                "UPDATE agent_runs SET session_id = ?2 WHERE id = ?1",
                params![run_id, session_id],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    pub fn set_status(&self, run_id: &str, status: RunStatus) -> Result<(), ActivityError> {
        let ended_at = if status.is_terminal() {
            Some(OffsetDateTime::now_utc().format(&Rfc3339)?)
        } else {
            None
        };
        self.storage.with_connection(|connection| {
            connection.execute(
                "UPDATE agent_runs
                 SET status = ?2, ended_at = COALESCE(?3, ended_at)
                 WHERE id = ?1",
                params![run_id, status.as_str(), ended_at],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    pub fn fail(&self, run_id: &str, error: &str) -> Result<(), ActivityError> {
        let ended_at = OffsetDateTime::now_utc().format(&Rfc3339)?;
        self.storage.with_connection(|connection| {
            connection.execute(
                "UPDATE agent_runs SET status = 'failed', error = ?2, ended_at = ?3 WHERE id = ?1",
                params![run_id, error, &ended_at],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    pub fn get(&self, run_id: &str) -> Result<RunRecord, ActivityError> {
        let row = self.storage.with_connection(|connection| {
            connection
                .query_row(RUN_SELECT_BY_ID, [run_id], decode_run_row)
                .optional()
        })?;
        row.ok_or_else(|| ActivityError::RunNotFound(run_id.to_owned()))
            .and_then(build_run)
    }

    pub fn list_for_task(&self, task_id: &str) -> Result<Vec<RunRecord>, ActivityError> {
        let rows = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(RUN_SELECT_BY_TASK)?;
            let mapped = statement.query_map([task_id], decode_run_row)?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()
        })?;
        rows.into_iter().map(build_run).collect()
    }

    /// Runs that were not terminal when they were last written — the candidates
    /// for crash recovery.
    pub fn unfinished(&self) -> Result<Vec<RunRecord>, ActivityError> {
        let rows = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(RUN_SELECT_UNFINISHED)?;
            let mapped = statement.query_map([], decode_run_row)?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()
        })?;
        rows.into_iter().map(build_run).collect()
    }
}

const RUN_SELECT_BY_ID: &str = concat!(
    "SELECT id, task_id, session_id, agent_id, adapter_kind, role, capabilities_json,",
    " context_pack_digest, status, last_sequence, error, started_at, ended_at",
    " FROM agent_runs WHERE id = ?1"
);

const RUN_SELECT_BY_TASK: &str = concat!(
    "SELECT id, task_id, session_id, agent_id, adapter_kind, role, capabilities_json,",
    " context_pack_digest, status, last_sequence, error, started_at, ended_at",
    " FROM agent_runs WHERE task_id = ?1 ORDER BY started_at, id"
);

const RUN_SELECT_UNFINISHED: &str = concat!(
    "SELECT id, task_id, session_id, agent_id, adapter_kind, role, capabilities_json,",
    " context_pack_digest, status, last_sequence, error, started_at, ended_at",
    " FROM agent_runs",
    " WHERE status IN ('starting', 'running', 'blocked', 'interrupting')",
    " ORDER BY started_at, id"
);

type RunRow = (
    String,
    String,
    Option<String>,
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    i64,
    Option<String>,
    String,
    Option<String>,
);

fn decode_run_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
    ))
}

fn build_run(row: RunRow) -> Result<RunRecord, ActivityError> {
    let (
        id,
        task_id,
        session_id,
        agent_id,
        adapter_kind,
        role,
        capabilities_json,
        context_pack_digest,
        status,
        last_sequence,
        error,
        started_at,
        ended_at,
    ) = row;
    Ok(RunRecord {
        id,
        task_id,
        session_id,
        agent_id,
        adapter_kind,
        role: RunRole::parse(&role)?,
        capabilities: serde_json::from_str(&capabilities_json)?,
        context_pack_digest,
        status: RunStatus::parse(&status)?,
        last_sequence,
        error,
        started_at,
        ended_at,
    })
}
