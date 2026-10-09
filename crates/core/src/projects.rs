use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mars_security::{RepositoryCapability, TRUSTED_REPOSITORY_CAPABILITIES, TrustDecision};
use mars_storage::Storage;
use mars_storage::projects::{ProjectRecord, ProjectRepository};
use serde::Serialize;
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryCandidate {
    pub name: String,
    pub root_path: PathBuf,
    pub git_common_dir: PathBuf,
    pub capabilities: Vec<RepositoryCapability>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSnapshot {
    pub project_id: String,
    pub repository_id: String,
    pub name: String,
    pub root_path: PathBuf,
    pub git_common_dir: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "status", content = "repository")]
pub enum OpenRepositoryOutcome {
    TrustRequired(RepositoryCandidate),
    Opened(ProjectSnapshot),
}

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("repository path is not a directory: {0}")]
    NotDirectory(PathBuf),
    #[error("directory is not a Git repository: {0}")]
    NotGitRepository(PathBuf),
    #[error("Git metadata file is malformed: {0}")]
    MalformedGitFile(PathBuf),
    #[error("repository path has no display name: {0}")]
    MissingName(PathBuf),
    #[error("filesystem operation failed for {path}: {source}")]
    Filesystem {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Storage(#[from] mars_storage::StorageError),
}

pub struct ProjectService {
    repository: ProjectRepository,
}

impl ProjectService {
    #[must_use]
    pub fn new(storage: Arc<Storage>) -> Self {
        Self {
            repository: ProjectRepository::new(storage),
        }
    }

    pub fn open_repository(
        &self,
        path: impl AsRef<Path>,
        trust: TrustDecision,
    ) -> Result<OpenRepositoryOutcome, ProjectError> {
        let candidate = inspect_repository(path.as_ref())?;
        if trust == TrustDecision::RequireConfirmation {
            return Ok(OpenRepositoryOutcome::TrustRequired(candidate));
        }

        let stored = self.repository.get_or_insert(
            &candidate.name,
            &candidate.root_path,
            &candidate.git_common_dir,
        )?;
        Ok(OpenRepositoryOutcome::Opened(stored.into()))
    }
}

impl From<ProjectRecord> for ProjectSnapshot {
    fn from(record: ProjectRecord) -> Self {
        Self {
            project_id: record.project_id,
            repository_id: record.repository_id,
            name: record.name,
            root_path: record.root_path,
            git_common_dir: record.git_common_dir,
        }
    }
}

fn inspect_repository(path: &Path) -> Result<RepositoryCandidate, ProjectError> {
    let root_path = canonicalize(path)?;
    if !root_path.is_dir() {
        return Err(ProjectError::NotDirectory(root_path));
    }
    let name = root_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .ok_or_else(|| ProjectError::MissingName(root_path.clone()))?;
    let git_entry = root_path.join(".git");
    let git_dir = if git_entry.is_dir() {
        canonicalize(&git_entry)?
    } else if git_entry.is_file() {
        resolve_git_file(&root_path, &git_entry)?
    } else {
        return Err(ProjectError::NotGitRepository(root_path));
    };
    let common_file = git_dir.join("commondir");
    let git_common_dir = if common_file.is_file() {
        let relative = read_trimmed(&common_file)?;
        canonicalize(&git_dir.join(relative))?
    } else {
        git_dir
    };

    Ok(RepositoryCandidate {
        name,
        root_path,
        git_common_dir,
        capabilities: TRUSTED_REPOSITORY_CAPABILITIES.to_vec(),
    })
}

fn resolve_git_file(root: &Path, git_file: &Path) -> Result<PathBuf, ProjectError> {
    let contents = read_trimmed(git_file)?;
    let Some(raw_path) = contents.strip_prefix("gitdir:") else {
        return Err(ProjectError::MalformedGitFile(git_file.into()));
    };
    let path = Path::new(raw_path.trim());
    canonicalize(
        if path.is_absolute() {
            path.into()
        } else {
            root.join(path)
        }
        .as_ref(),
    )
}

fn read_trimmed(path: &Path) -> Result<String, ProjectError> {
    fs::read_to_string(path)
        .map(|contents| contents.trim().to_owned())
        .map_err(|source| ProjectError::Filesystem {
            path: path.into(),
            source,
        })
}

/// Resolves a path the way Mars stores it: absolute, symlinks followed, and on
/// Windows in the ordinary `C:\` form rather than the verbatim `\\?\C:\` one
/// whenever the ordinary form can express it. The verbatim prefix is what
/// `std::fs::canonicalize` returns there, and Git, agent CLIs and people all
/// read it worse than the path they chose.
pub fn canonicalize(path: &Path) -> Result<PathBuf, ProjectError> {
    dunce::canonicalize(path).map_err(|source| ProjectError::Filesystem {
        path: path.into(),
        source,
    })
}
