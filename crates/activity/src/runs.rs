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
    /// The run whose provider session this one continues, for a follow-up.
    pub resumes_run_id: Option<String>,
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
    /// The provider's own id for the conversation, which a follow-up resumes.
    pub provider_session_id: Option<String>,
    pub resumes_run_id: Option<String>,
    /// What the provider reported the run cost.
    pub usage: Option<Value>,
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
                      context_pack_digest, status, last_sequence, error, started_at, ended_at,
                      resumes_run_id)
                 VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, 'starting', 0, NULL, ?8, NULL, ?9)",
                params![
                    &id,
                    &run.task_id,
                    &run.agent_id,
                    &run.adapter_kind,
                    run.role.as_str(),
                    &capabilities,
                    run.context_pack_digest.as_deref(),
                    &started_at,
                    run.resumes_run_id.as_deref(),
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

    /// Remember the provider's session id, so a follow-up can resume it.
    pub fn set_provider_session(
        &self,
        run_id: &str,
        provider_session_id: &str,
    ) -> Result<(), ActivityError> {
        self.storage.with_connection(|connection| {
            connection.execute(
                "UPDATE agent_runs SET provider_session_id = ?2 WHERE id = ?1",
                params![run_id, provider_session_id],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    /// Record what the provider said the run cost.
    pub fn set_usage(&self, run_id: &str, usage: &Value) -> Result<(), ActivityError> {
        let usage = serde_json::to_string(usage)?;
        self.storage.with_connection(|connection| {
            connection.execute(
                "UPDATE agent_runs SET usage_json = ?2 WHERE id = ?1",
                params![run_id, usage],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    /// The run that owns `session_id`, the supervisor's id for its process.
    pub fn for_session(&self, session_id: &str) -> Result<Option<RunRecord>, ActivityError> {
        let row = self.storage.with_connection(|connection| {
            connection
                .query_row(
                    &format!("{RUN_COLUMNS} WHERE session_id = ?1"),
                    [session_id],
                    decode_run_row,
                )
                .optional()
        })?;
        row.map(build_run).transpose()
    }

    /// The latest run of a task that left a session to resume.
    pub fn latest_resumable(&self, task_id: &str) -> Result<Option<RunRecord>, ActivityError> {
        let row = self.storage.with_connection(|connection| {
            connection
                .query_row(
                    &format!(
                        "{RUN_COLUMNS} WHERE task_id = ?1 AND provider_session_id IS NOT NULL
                         ORDER BY started_at DESC, id DESC LIMIT 1"
                    ),
                    [task_id],
                    decode_run_row,
                )
                .optional()
        })?;
        row.map(build_run).transpose()
    }

    /// The most recent runs across every task of a repository, newest first.
    pub fn recent_for_repository(
        &self,
        repository_id: &str,
        limit: usize,
    ) -> Result<Vec<RunRecord>, ActivityError> {
        let rows = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(&format!(
                "{RUN_COLUMNS} WHERE task_id IN (SELECT id FROM tasks WHERE repository_id = ?1)
                 ORDER BY started_at DESC, id DESC LIMIT ?2"
            ))?;
            let mapped = statement.query_map(
                params![repository_id, i64::try_from(limit).unwrap_or(i64::MAX)],
                decode_run_row,
            )?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()
        })?;
        rows.into_iter().map(build_run).collect()
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
                .query_row(
                    &format!("{RUN_COLUMNS} WHERE id = ?1"),
                    [run_id],
                    decode_run_row,
                )
                .optional()
        })?;
        row.ok_or_else(|| ActivityError::RunNotFound(run_id.to_owned()))
            .and_then(build_run)
    }

    pub fn list_for_task(&self, task_id: &str) -> Result<Vec<RunRecord>, ActivityError> {
        let rows = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(&format!(
                "{RUN_COLUMNS} WHERE task_id = ?1 ORDER BY started_at, id"
            ))?;
            let mapped = statement.query_map([task_id], decode_run_row)?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()
        })?;
        rows.into_iter().map(build_run).collect()
    }

    /// Runs that were not terminal when they were last written — the candidates
    /// for crash recovery.
    pub fn unfinished(&self) -> Result<Vec<RunRecord>, ActivityError> {
        let rows = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(&format!(
                "{RUN_COLUMNS} WHERE status IN ('starting', 'running', 'blocked', 'interrupting')
                 ORDER BY started_at, id"
            ))?;
            let mapped = statement.query_map([], decode_run_row)?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()
        })?;
        rows.into_iter().map(build_run).collect()
    }
}

const RUN_COLUMNS: &str = concat!(
    "SELECT id, task_id, session_id, agent_id, adapter_kind, role, capabilities_json,",
    " context_pack_digest, status, last_sequence, error, started_at, ended_at,",
    " provider_session_id, resumes_run_id, usage_json",
    " FROM agent_runs"
);

struct RunRow {
    id: String,
    task_id: String,
    session_id: Option<String>,
    agent_id: String,
    adapter_kind: String,
    role: String,
    capabilities_json: String,
    context_pack_digest: Option<String>,
    status: String,
    last_sequence: i64,
    error: Option<String>,
    started_at: String,
    ended_at: Option<String>,
    provider_session_id: Option<String>,
    resumes_run_id: Option<String>,
    usage_json: Option<String>,
}

fn decode_run_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunRow> {
    Ok(RunRow {
        id: row.get(0)?,
        task_id: row.get(1)?,
        session_id: row.get(2)?,
        agent_id: row.get(3)?,
        adapter_kind: row.get(4)?,
        role: row.get(5)?,
        capabilities_json: row.get(6)?,
        context_pack_digest: row.get(7)?,
        status: row.get(8)?,
        last_sequence: row.get(9)?,
        error: row.get(10)?,
        started_at: row.get(11)?,
        ended_at: row.get(12)?,
        provider_session_id: row.get(13)?,
        resumes_run_id: row.get(14)?,
        usage_json: row.get(15)?,
    })
}

fn build_run(row: RunRow) -> Result<RunRecord, ActivityError> {
    Ok(RunRecord {
        id: row.id,
        task_id: row.task_id,
        session_id: row.session_id,
        agent_id: row.agent_id,
        adapter_kind: row.adapter_kind,
        role: RunRole::parse(&row.role)?,
        capabilities: serde_json::from_str(&row.capabilities_json)?,
        context_pack_digest: row.context_pack_digest,
        status: RunStatus::parse(&row.status)?,
        last_sequence: row.last_sequence,
        error: row.error,
        started_at: row.started_at,
        ended_at: row.ended_at,
        provider_session_id: row.provider_session_id,
        resumes_run_id: row.resumes_run_id,
        usage: row
            .usage_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?,
    })
}
