pub mod javascript;
pub mod publish;
pub mod walk;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mars_storage::Storage;
use mars_storage::projects::ProjectRepository;
use rusqlite::params;
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use javascript::ParsedFile;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexSummary {
    pub run_id: String,
    pub indexed_files: usize,
    pub skipped_files: usize,
    pub deleted_files: usize,
}

#[derive(Debug, Error)]
pub enum IndexError {
    #[error(transparent)]
    Storage(#[from] mars_storage::StorageError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("failed to format an index timestamp: {0}")]
    Timestamp(#[from] time::error::Format),
    #[error("filesystem operation failed for {path}: {source}")]
    Filesystem {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("path is outside the repository: {0}")]
    OutsideRepository(PathBuf),
    #[error("repository traversal failed: {0}")]
    Walk(String),
    #[error("source parsing failed: {0}")]
    Parse(String),
    #[error("index run `{run_id}` was cancelled")]
    Cancelled { run_id: String },
}

pub struct Indexer {
    storage: Arc<Storage>,
    projects: ProjectRepository,
}

impl Indexer {
    #[must_use]
    pub fn new(storage: Arc<Storage>) -> Self {
        Self {
            projects: ProjectRepository::new(storage.clone()),
            storage,
        }
    }

    pub fn index_repository(
        &self,
        repository_id: &str,
        cancellation: &CancellationToken,
    ) -> Result<IndexSummary, IndexError> {
        self.index_repository_observing(repository_id, cancellation, |_, _| {})
    }

    pub fn index_repository_observing(
        &self,
        repository_id: &str,
        cancellation: &CancellationToken,
        mut observer: impl FnMut(usize, &Path),
    ) -> Result<IndexSummary, IndexError> {
        let repository = self.projects.find_repository(repository_id)?;
        let run_id = Uuid::now_v7().to_string();
        let started_at = timestamp()?;
        self.storage.with_connection(|connection| {
            connection.execute(
                "INSERT INTO index_runs (id, repository_id, status, started_at) VALUES (?1, ?2, 'running', ?3)",
                params![run_id, repository_id, started_at],
            )?;
            Ok(())
        })?;

        let result = self.prepare(
            repository_id,
            &repository.root_path,
            &run_id,
            cancellation,
            &mut observer,
        );
        match result {
            Ok(summary) => Ok(summary),
            Err(IndexError::Cancelled { .. }) => {
                self.finish_run(&run_id, "interrupted", None)?;
                Err(IndexError::Cancelled { run_id })
            }
            Err(error) => {
                self.finish_run(&run_id, "failed", Some(&error.to_string()))?;
                Err(error)
            }
        }
    }

    fn prepare(
        &self,
        repository_id: &str,
        root: &Path,
        run_id: &str,
        cancellation: &CancellationToken,
        observer: &mut impl FnMut(usize, &Path),
    ) -> Result<IndexSummary, IndexError> {
        let existing: HashMap<String, String> = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT relative_path, content_hash FROM files WHERE repository_id = ?1 AND deleted_at IS NULL",
            )?;
            statement
                .query_map([repository_id], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })?;
        let sources = walk::source_files(root)?;
        let seen: HashSet<_> = sources
            .iter()
            .map(|source| source.relative_path.clone())
            .collect();
        let deleted: Vec<_> = existing
            .keys()
            .filter(|path| !seen.contains(*path))
            .cloned()
            .collect();
        let mut parsed: Vec<ParsedFile> = Vec::new();
        let mut skipped = 0;

        for (index, source) in sources.into_iter().enumerate() {
            observer(index + 1, Path::new(&source.relative_path));
            if cancellation.is_cancelled() {
                return Err(IndexError::Cancelled {
                    run_id: run_id.into(),
                });
            }
            if existing
                .get(&source.relative_path)
                .is_some_and(|hash| hash == &source.content_hash)
            {
                skipped += 1;
            } else {
                parsed.push(javascript::parse(source)?);
            }
        }

        if cancellation.is_cancelled() {
            return Err(IndexError::Cancelled {
                run_id: run_id.into(),
            });
        }
        publish::publish(
            &self.storage,
            repository_id,
            run_id,
            &parsed,
            &deleted,
            seen.len(),
        )?;
        Ok(IndexSummary {
            run_id: run_id.into(),
            indexed_files: parsed.len(),
            skipped_files: skipped,
            deleted_files: deleted.len(),
        })
    }

    fn finish_run(
        &self,
        run_id: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), IndexError> {
        let finished_at = timestamp()?;
        self.storage.with_connection(|connection| {
            connection.execute(
                "UPDATE index_runs SET status = ?2, error = ?3, finished_at = ?4 WHERE id = ?1",
                params![run_id, status, error, finished_at],
            )?;
            Ok(())
        })?;
        Ok(())
    }
}

pub(crate) fn timestamp() -> Result<String, IndexError> {
    Ok(OffsetDateTime::now_utc().format(&Rfc3339)?)
}
