//! Durable local storage for Mars.

pub mod jobs;
pub mod migrations;
pub mod projects;

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::Connection;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("stored migration {version} does not match its source checksum")]
    MigrationChecksum { version: u32 },
    #[error("failed to format a storage timestamp: {0}")]
    Timestamp(#[from] time::error::Format),
    #[error("failed to encode or decode stored JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("job `{0}` does not exist")]
    JobNotFound(String),
    #[error("job `{job_id}` cannot transition from {from} to {to}")]
    InvalidJobTransition {
        job_id: String,
        from: String,
        to: String,
    },
    #[error("{0} lock was poisoned")]
    StatePoisoned(&'static str),
}

pub type Result<T> = std::result::Result<T, StorageError>;

pub struct Storage {
    connection: Mutex<Connection>,
}

impl Storage {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let mut connection = Connection::open(path)?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.busy_timeout(Duration::from_secs(5))?;
        migrations::apply_all(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn with_connection<T>(
        &self,
        operation: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> Result<T> {
        let connection = self.connection()?;
        Ok(operation(&connection)?)
    }

    pub(crate) fn with_connection_mut<T>(
        &self,
        operation: impl FnOnce(&mut Connection) -> Result<T>,
    ) -> Result<T> {
        let mut connection = self.connection()?;
        operation(&mut connection)
    }

    pub(crate) fn connection(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| StorageError::StatePoisoned("storage connection"))
    }
}

pub(crate) fn now_utc() -> Result<String> {
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;
    Ok(OffsetDateTime::now_utc().format(&Rfc3339)?)
}
