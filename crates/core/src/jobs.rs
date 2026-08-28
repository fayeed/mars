use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use mars_storage::jobs::JobRepository;
pub use mars_storage::jobs::{JobEvent, JobStatus};
use mars_storage::{Result, Storage, StorageError};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

pub struct JobRegistry {
    repository: JobRepository,
    tokens: Mutex<HashMap<String, CancellationToken>>,
}

impl JobRegistry {
    #[must_use]
    pub fn new(storage: Arc<Storage>) -> Self {
        Self {
            repository: JobRepository::new(storage),
            tokens: Mutex::new(HashMap::new()),
        }
    }

    pub fn create(&self, id: &str, kind: &str) -> Result<()> {
        self.repository.create(id, kind)
    }

    pub fn start(&self, id: &str) -> Result<CancellationToken> {
        let mut tokens = self
            .tokens
            .lock()
            .map_err(|_| StorageError::StatePoisoned("job cancellation registry"))?;
        self.repository
            .transition(id, &[JobStatus::Queued], JobStatus::Running)?;
        let token = CancellationToken::new();
        tokens.insert(id.into(), token.clone());
        Ok(token)
    }

    pub fn cancel(&self, id: &str) -> Result<()> {
        let tokens = self
            .tokens
            .lock()
            .map_err(|_| StorageError::StatePoisoned("job cancellation registry"))?;
        self.repository.transition(
            id,
            &[JobStatus::Queued, JobStatus::Running],
            JobStatus::CancelRequested,
        )?;
        if let Some(token) = tokens.get(id) {
            token.cancel();
        }
        Ok(())
    }

    pub fn complete(&self, id: &str) -> Result<()> {
        self.repository.transition(
            id,
            &[JobStatus::Running, JobStatus::CancelRequested],
            JobStatus::Completed,
        )?;
        self.remove_token(id)
    }

    pub fn fail(&self, id: &str) -> Result<()> {
        self.repository.transition(
            id,
            &[JobStatus::Running, JobStatus::CancelRequested],
            JobStatus::Failed,
        )?;
        self.remove_token(id)
    }

    pub fn status(&self, id: &str) -> Result<JobStatus> {
        self.repository.status(id)
    }

    pub fn append_event(&self, id: &str, event_type: &str, payload: Value) -> Result<i64> {
        self.repository.append_event(id, event_type, &payload)
    }

    pub fn events(&self, id: &str) -> Result<Vec<JobEvent>> {
        self.repository.events(id)
    }

    pub fn recover_interrupted(&self) -> Result<Vec<String>> {
        let recovered = self.repository.recover_interrupted()?;
        let mut tokens = self
            .tokens
            .lock()
            .map_err(|_| StorageError::StatePoisoned("job cancellation registry"))?;
        for id in &recovered {
            tokens.remove(id);
        }
        Ok(recovered)
    }

    fn remove_token(&self, id: &str) -> Result<()> {
        self.tokens
            .lock()
            .map_err(|_| StorageError::StatePoisoned("job cancellation registry"))?
            .remove(id);
        Ok(())
    }
}
