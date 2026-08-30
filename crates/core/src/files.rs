use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use ignore::WalkBuilder;
use mars_security::{is_default_ignored_directory, is_sensitive_filename};
use mars_storage::Storage;
use mars_storage::projects::ProjectRepository;
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

pub const MAX_EDITABLE_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_PAGE_SIZE: usize = 1_000;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub relative_path: String,
    pub size: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePage {
    pub entries: Vec<FileEntry>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDocument {
    pub relative_path: String,
    pub content: String,
    pub version: String,
    pub size: u64,
}

#[derive(Debug, Error)]
pub enum FileError {
    #[error(transparent)]
    Storage(#[from] mars_storage::StorageError),
    #[error("invalid repository-relative path: {0}")]
    InvalidRelativePath(String),
    #[error("path is outside the trusted repository: {0}")]
    OutsideRepository(PathBuf),
    #[error("path is ignored by repository policy: {0}")]
    IgnoredPath(String),
    #[error("path is not a regular file: {0}")]
    NotFile(PathBuf),
    #[error("file is binary and cannot be edited as text: {0}")]
    BinaryFile(PathBuf),
    #[error("file is {size} bytes; the editor limit is {limit} bytes: {path}")]
    FileTooLarge {
        path: PathBuf,
        size: u64,
        limit: u64,
    },
    #[error("file changed on disk (expected {expected}, found {actual}): {path}")]
    WriteConflict {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    #[error("file traversal failed: {0}")]
    Walk(String),
    #[error("filesystem operation failed for {path}: {source}")]
    Filesystem {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub struct FileService {
    projects: ProjectRepository,
}

impl FileService {
    #[must_use]
    pub fn new(storage: Arc<Storage>) -> Self {
        Self {
            projects: ProjectRepository::new(storage),
        }
    }

    pub fn list_files(
        &self,
        repository_id: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<FilePage, FileError> {
        let repository = self.projects.find_repository(repository_id)?;
        let mut entries = walk_files(&repository.root_path)?;
        entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        if let Some(cursor) = cursor {
            entries.retain(|entry| entry.relative_path.as_str() > cursor);
        }

        let limit = limit.clamp(1, MAX_PAGE_SIZE);
        let has_more = entries.len() > limit;
        entries.truncate(limit);
        let next_cursor = has_more
            .then(|| entries.last().map(|entry| entry.relative_path.clone()))
            .flatten();
        Ok(FilePage {
            entries,
            next_cursor,
        })
    }

    pub fn read_file(
        &self,
        repository_id: &str,
        relative_path: &str,
    ) -> Result<FileDocument, FileError> {
        validate_relative_path(relative_path)?;
        let repository = self.projects.find_repository(repository_id)?;
        if !walk_files(&repository.root_path)?
            .iter()
            .any(|entry| entry.relative_path == relative_path)
        {
            return Err(FileError::IgnoredPath(relative_path.to_owned()));
        }
        let path = resolve_file(&repository.root_path, relative_path)?;
        read_document(&path, relative_path)
    }

    pub fn write_file(
        &self,
        repository_id: &str,
        relative_path: &str,
        content: &str,
        expected_version: &str,
    ) -> Result<FileDocument, FileError> {
        if content.len() as u64 > MAX_EDITABLE_FILE_BYTES {
            return Err(FileError::FileTooLarge {
                path: PathBuf::from(relative_path),
                size: content.len() as u64,
                limit: MAX_EDITABLE_FILE_BYTES,
            });
        }
        let repository = self.projects.find_repository(repository_id)?;
        let current = self.read_file(repository_id, relative_path)?;
        let path = resolve_file(&repository.root_path, relative_path)?;
        if current.version != expected_version {
            return Err(FileError::WriteConflict {
                path,
                expected: expected_version.to_owned(),
                actual: current.version,
            });
        }

        atomic_replace(&path, content.as_bytes())?;
        read_document(&path, relative_path)
    }
}

fn walk_files(root: &Path) -> Result<Vec<FileEntry>, FileError> {
    let mut builder = WalkBuilder::new(root);
    builder.add_custom_ignore_filename(".marsignore");
    builder.hidden(false);
    builder.filter_entry(|entry| {
        entry
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .is_none_or(|name| !is_default_ignored_directory(name) && !is_sensitive_filename(name))
    });

    let mut files = Vec::new();
    for result in builder.build() {
        let entry = result.map_err(|error| FileError::Walk(error.to_string()))?;
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|_| FileError::OutsideRepository(path.into()))?;
        let relative_path = relative.to_string_lossy().replace('\\', "/");
        let size = entry
            .metadata()
            .map_err(|error| FileError::Walk(error.to_string()))?
            .len();
        files.push(FileEntry {
            relative_path,
            size,
        });
    }
    Ok(files)
}

fn resolve_file(root: &Path, relative_path: &str) -> Result<PathBuf, FileError> {
    validate_relative_path(relative_path)?;
    let relative = Path::new(relative_path);
    let unresolved = root.join(relative);
    let path = fs::canonicalize(&unresolved).map_err(|source| FileError::Filesystem {
        path: unresolved,
        source,
    })?;
    if !path.starts_with(root) {
        return Err(FileError::OutsideRepository(path));
    }
    if !path.is_file() {
        return Err(FileError::NotFile(path));
    }
    Ok(path)
}

fn validate_relative_path(relative_path: &str) -> Result<(), FileError> {
    let relative = Path::new(relative_path);
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(FileError::InvalidRelativePath(relative_path.to_owned()));
    }
    Ok(())
}

fn read_document(path: &Path, relative_path: &str) -> Result<FileDocument, FileError> {
    let size = fs::metadata(path)
        .map_err(|source| FileError::Filesystem {
            path: path.into(),
            source,
        })?
        .len();
    if size > MAX_EDITABLE_FILE_BYTES {
        return Err(FileError::FileTooLarge {
            path: path.into(),
            size,
            limit: MAX_EDITABLE_FILE_BYTES,
        });
    }
    let bytes = fs::read(path).map_err(|source| FileError::Filesystem {
        path: path.into(),
        source,
    })?;
    if bytes.contains(&0) {
        return Err(FileError::BinaryFile(path.into()));
    }
    let content = String::from_utf8(bytes).map_err(|_| FileError::BinaryFile(path.into()))?;
    let version = digest(content.as_bytes());
    Ok(FileDocument {
        relative_path: relative_path.to_owned(),
        size,
        content,
        version,
    })
}

fn atomic_replace(path: &Path, content: &[u8]) -> Result<(), FileError> {
    let parent = path
        .parent()
        .ok_or_else(|| FileError::InvalidRelativePath(path.display().to_string()))?;
    let temporary = parent.join(format!(".mars-save-{}.tmp", Uuid::now_v7()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|source| FileError::Filesystem {
                path: temporary.clone(),
                source,
            })?;
        file.write_all(content)
            .and_then(|()| file.sync_all())
            .map_err(|source| FileError::Filesystem {
                path: temporary.clone(),
                source,
            })?;
        let permissions = fs::metadata(path)
            .map_err(|source| FileError::Filesystem {
                path: path.into(),
                source,
            })?
            .permissions();
        fs::set_permissions(&temporary, permissions).map_err(|source| FileError::Filesystem {
            path: temporary.clone(),
            source,
        })?;
        fs::rename(&temporary, path).map_err(|source| FileError::Filesystem {
            path: path.into(),
            source,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn digest(content: &[u8]) -> String {
    format!("{:x}", Sha256::digest(content))
}
