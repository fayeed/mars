use std::fmt;
use std::sync::Arc;

use rusqlite::{OptionalExtension, params};
use serde_json::Value;

use crate::{Result, Storage, StorageError, now_utc};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobStatus {
    Queued,
    Running,
    CancelRequested,
    Completed,
    Failed,
    Interrupted,
}

impl JobStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::CancelRequested => "cancel_requested",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        }
    }

    fn parse(value: &str) -> rusqlite::Result<Self> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "cancel_requested" => Ok(Self::CancelRequested),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "interrupted" => Ok(Self::Interrupted),
            _ => Err(rusqlite::Error::InvalidColumnType(
                0,
                "status".into(),
                rusqlite::types::Type::Text,
            )),
        }
    }
}

impl fmt::Display for JobStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct JobEvent {
    pub sequence: i64,
    pub event_type: String,
    pub payload: Value,
    pub occurred_at: String,
}

pub struct JobRepository {
    storage: Arc<Storage>,
}

impl JobRepository {
    #[must_use]
    pub fn new(storage: Arc<Storage>) -> Self {
        Self { storage }
    }

    pub fn create(&self, id: &str, kind: &str) -> Result<()> {
        let timestamp = now_utc()?;
        self.storage.with_connection(|connection| {
            connection.execute(
                "INSERT INTO jobs (id, kind, status, created_at, updated_at)
                 VALUES (?1, ?2, 'queued', ?3, ?3)",
                params![id, kind, timestamp],
            )?;
            Ok(())
        })
    }

    pub fn status(&self, id: &str) -> Result<JobStatus> {
        self.storage
            .with_connection(|connection| {
                connection
                    .query_row("SELECT status FROM jobs WHERE id = ?1", [id], |row| {
                        let status: String = row.get(0)?;
                        JobStatus::parse(&status)
                    })
                    .optional()
            })?
            .ok_or_else(|| StorageError::JobNotFound(id.into()))
    }

    pub fn transition(&self, id: &str, from: &[JobStatus], to: JobStatus) -> Result<()> {
        let current = self.status(id)?;
        if !from.contains(&current) {
            return Err(StorageError::InvalidJobTransition {
                job_id: id.into(),
                from: current.to_string(),
                to: to.to_string(),
            });
        }

        let timestamp = now_utc()?;
        let started_at = (to == JobStatus::Running).then_some(timestamp.as_str());
        let finished_at = matches!(
            to,
            JobStatus::Completed | JobStatus::Failed | JobStatus::Interrupted
        )
        .then_some(timestamp.as_str());
        let changed = self.storage.with_connection(|connection| {
            connection.execute(
                "UPDATE jobs
                 SET status = ?1,
                     updated_at = ?2,
                     started_at = COALESCE(started_at, ?3),
                     finished_at = COALESCE(?4, finished_at)
                 WHERE id = ?5 AND status = ?6",
                params![
                    to.as_str(),
                    timestamp,
                    started_at,
                    finished_at,
                    id,
                    current.as_str()
                ],
            )
        })?;

        if changed == 1 {
            Ok(())
        } else {
            Err(StorageError::InvalidJobTransition {
                job_id: id.into(),
                from: current.to_string(),
                to: to.to_string(),
            })
        }
    }

    pub fn append_event(&self, id: &str, event_type: &str, payload: &Value) -> Result<i64> {
        let mut connection = self.storage.connection()?;
        let transaction = connection.transaction()?;
        let sequence: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(sequence), 0) + 1 FROM job_events WHERE job_id = ?1",
            [id],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT INTO job_events (job_id, sequence, event_type, payload_json, occurred_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, sequence, event_type, payload.to_string(), now_utc()?],
        )?;
        transaction.commit()?;
        Ok(sequence)
    }

    pub fn events(&self, id: &str) -> Result<Vec<JobEvent>> {
        self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT sequence, event_type, payload_json, occurred_at
                 FROM job_events WHERE job_id = ?1 ORDER BY sequence",
            )?;
            let rows = statement.query_map([id], |row| {
                let payload: String = row.get(2)?;
                let payload = serde_json::from_str(&payload).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;
                Ok(JobEvent {
                    sequence: row.get(0)?,
                    event_type: row.get(1)?,
                    payload,
                    occurred_at: row.get(3)?,
                })
            })?;
            rows.collect()
        })
    }

    pub fn recover_interrupted(&self) -> Result<Vec<String>> {
        let timestamp = now_utc()?;
        let mut connection = self.storage.connection()?;
        let transaction = connection.transaction()?;
        let ids = {
            let mut statement = transaction.prepare(
                "SELECT id FROM jobs
                 WHERE status IN ('running', 'cancel_requested') ORDER BY created_at, id",
            )?;
            statement
                .query_map([], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?
        };
        for id in &ids {
            transaction.execute(
                "UPDATE jobs
                 SET status = 'interrupted', updated_at = ?1, finished_at = ?1
                 WHERE id = ?2 AND status IN ('running', 'cancel_requested')",
                params![timestamp, id],
            )?;
        }
        transaction.commit()?;
        Ok(ids)
    }
}
