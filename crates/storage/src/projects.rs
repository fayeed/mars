use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use crate::{Result, Storage, now_utc};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectRecord {
    pub project_id: String,
    pub repository_id: String,
    pub name: String,
    pub root_path: PathBuf,
    pub git_common_dir: PathBuf,
}

/// Cloneable: it holds a handle to the database rather than the database, so
/// every clone reads and writes the same one.
#[derive(Clone)]
pub struct ProjectRepository {
    storage: Arc<Storage>,
}

impl ProjectRepository {
    #[must_use]
    pub fn new(storage: Arc<Storage>) -> Self {
        Self { storage }
    }

    pub fn get_or_insert(
        &self,
        name: &str,
        root_path: &Path,
        git_common_dir: &Path,
    ) -> Result<ProjectRecord> {
        let root_text = root_path.to_string_lossy().into_owned();
        let common_text = git_common_dir.to_string_lossy().into_owned();

        self.storage.with_connection_mut(|connection| {
            let transaction = connection.transaction()?;
            if let Some(existing) = find_by_root(&transaction, &root_text)? {
                transaction.commit()?;
                return Ok(existing);
            }

            let record = ProjectRecord {
                project_id: Uuid::now_v7().to_string(),
                repository_id: Uuid::now_v7().to_string(),
                name: name.into(),
                root_path: root_path.into(),
                git_common_dir: git_common_dir.into(),
            };
            let timestamp = now_utc()?;
            transaction.execute(
                "INSERT INTO projects (id, name, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
                params![record.project_id, record.name, timestamp],
            )?;
            transaction.execute(
                "INSERT INTO repositories
                 (id, project_id, root_path, git_common_dir, state, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?5)",
                params![
                    record.repository_id,
                    record.project_id,
                    root_text,
                    common_text,
                    timestamp
                ],
            )?;
            transaction.commit()?;
            Ok(record.clone())
        })
    }

    pub fn find_repository(&self, repository_id: &str) -> Result<ProjectRecord> {
        self.storage
            .with_connection(|connection| find_by_id(connection, repository_id))?
            .ok_or_else(|| crate::StorageError::RepositoryNotFound(repository_id.to_owned()))
    }
}

fn find_by_id(
    connection: &Connection,
    repository_id: &str,
) -> rusqlite::Result<Option<ProjectRecord>> {
    connection
        .query_row(
            "SELECT p.id, r.id, p.name, r.root_path, r.git_common_dir
             FROM repositories r
             JOIN projects p ON p.id = r.project_id
             WHERE r.id = ?1",
            [repository_id],
            |row| {
                Ok(ProjectRecord {
                    project_id: row.get(0)?,
                    repository_id: row.get(1)?,
                    name: row.get(2)?,
                    root_path: PathBuf::from(row.get::<_, String>(3)?),
                    git_common_dir: PathBuf::from(row.get::<_, String>(4)?),
                })
            },
        )
        .optional()
}

fn find_by_root(
    connection: &Connection,
    root_path: &str,
) -> rusqlite::Result<Option<ProjectRecord>> {
    connection
        .query_row(
            "SELECT p.id, r.id, p.name, r.root_path, r.git_common_dir
             FROM repositories r
             JOIN projects p ON p.id = r.project_id
             WHERE r.root_path = ?1",
            [root_path],
            |row| {
                Ok(ProjectRecord {
                    project_id: row.get(0)?,
                    repository_id: row.get(1)?,
                    name: row.get(2)?,
                    root_path: PathBuf::from(row.get::<_, String>(3)?),
                    git_common_dir: PathBuf::from(row.get::<_, String>(4)?),
                })
            },
        )
        .optional()
}
