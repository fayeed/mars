use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mars_storage::{Storage, StorageError};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::artifacts::{StoredArtifact, encode_digest, store, store_bytes, verify};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextKind {
    Text,
    Image,
    File,
}

impl ContextKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
            Self::File => "file",
        }
    }

    fn parse(value: &str) -> Result<Self, CaptureError> {
        match value {
            "text" => Ok(Self::Text),
            "image" => Ok(Self::Image),
            "file" => Ok(Self::File),
            other => Err(CaptureError::InvalidStoredKind(other.to_owned())),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextItem {
    pub id: String,
    pub repository_id: String,
    pub kind: ContextKind,
    pub title: String,
    pub body_text: Option<String>,
    pub original_uri: Option<String>,
    pub mime_type: String,
    pub content_hash: String,
    pub classification: Option<String>,
    pub quarantine_reason: Option<String>,
    pub artifact_path: Option<PathBuf>,
    pub original_name: Option<String>,
    pub captured_at: String,
}

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("filesystem operation failed for `{path}`: {source}")]
    Filesystem {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("context artifact is not a regular file: {0}")]
    NotAFile(PathBuf),
    #[error("context artifact is {bytes} bytes; maximum is {maximum}")]
    ArtifactTooLarge { bytes: usize, maximum: usize },
    #[error(
        "captured artifact `{path}` failed digest verification: expected {expected}, got {actual}"
    )]
    DigestMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    #[error("text context cannot be empty")]
    EmptyText,
    #[error("stored context kind is invalid: {0}")]
    InvalidStoredKind(String),
    #[error("context item `{0}` does not exist")]
    ItemNotFound(String),
    #[error("quarantined context cannot be attached: {0}")]
    Quarantined(String),
    #[error("failed to format context timestamp: {0}")]
    Timestamp(#[from] time::error::Format),
}

#[derive(Clone)]
pub struct ContextService {
    storage: Arc<Storage>,
    artifact_root: PathBuf,
}

struct NewContext<'a> {
    repository_id: &'a str,
    task_id: Option<&'a str>,
    kind: ContextKind,
    title: &'a str,
    body_text: Option<&'a str>,
    original_uri: Option<&'a str>,
    mime_type: &'a str,
    content_hash: &'a str,
    quarantine_reason: Option<String>,
    artifact: Option<(&'a StoredArtifact, &'a str)>,
}

impl ContextService {
    #[must_use]
    pub fn new(storage: Arc<Storage>, artifact_root: impl AsRef<Path>) -> Self {
        Self {
            storage,
            artifact_root: artifact_root.as_ref().to_owned(),
        }
    }

    pub fn capture_text(
        &self,
        repository_id: &str,
        task_id: Option<&str>,
        title: Option<&str>,
        text: &str,
    ) -> Result<ContextItem, CaptureError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(CaptureError::EmptyText);
        }
        let digest = encode_digest(text.as_bytes());
        let quarantine_reason = detect_secret(text);
        self.insert(NewContext {
            repository_id,
            task_id,
            kind: ContextKind::Text,
            title: title
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("Captured text"),
            body_text: Some(text),
            original_uri: None,
            mime_type: "text/plain",
            content_hash: &digest,
            quarantine_reason,
            artifact: None,
        })
    }

    pub fn capture_file(
        &self,
        repository_id: &str,
        task_id: Option<&str>,
        title: Option<&str>,
        source: &Path,
        mime_type: &str,
    ) -> Result<ContextItem, CaptureError> {
        let artifact = store(source, &self.artifact_root)?;
        let original_name = source
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| "artifact".into());
        let kind = if mime_type.starts_with("image/") {
            ContextKind::Image
        } else {
            ContextKind::File
        };
        let quarantine_reason = scan_file_for_secret(source, mime_type)?;
        let original_uri = format!("file://{}", source.display());
        let result = self.insert(NewContext {
            repository_id,
            task_id,
            kind,
            title: title
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(&original_name),
            body_text: None,
            original_uri: Some(&original_uri),
            mime_type,
            content_hash: &artifact.digest,
            quarantine_reason,
            artifact: Some((&artifact, &original_name)),
        });
        if result.is_err() && artifact.created {
            let _ = fs::remove_file(&artifact.path);
        }
        result
    }

    pub fn capture_bytes(
        &self,
        repository_id: &str,
        task_id: Option<&str>,
        title: Option<&str>,
        original_name: &str,
        bytes: &[u8],
        mime_type: &str,
    ) -> Result<ContextItem, CaptureError> {
        let artifact = store_bytes(bytes, &self.artifact_root)?;
        let kind = if mime_type.starts_with("image/") {
            ContextKind::Image
        } else {
            ContextKind::File
        };
        let quarantine_reason = if mime_type.starts_with("text/") {
            std::str::from_utf8(bytes).ok().and_then(detect_secret)
        } else {
            None
        };
        let result = self.insert(NewContext {
            repository_id,
            task_id,
            kind,
            title: title
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(original_name),
            body_text: None,
            original_uri: None,
            mime_type,
            content_hash: &artifact.digest,
            quarantine_reason,
            artifact: Some((&artifact, original_name)),
        });
        if result.is_err() && artifact.created {
            let _ = fs::remove_file(&artifact.path);
        }
        result
    }

    pub fn classify(
        &self,
        item_id: &str,
        classification: &str,
    ) -> Result<ContextItem, CaptureError> {
        let timestamp = now()?;
        self.storage.with_connection(|connection| {
            let changed = connection.execute(
                "UPDATE context_items SET classification = ?2, updated_at = ?3 WHERE id = ?1",
                params![item_id, classification, timestamp],
            )?;
            if changed == 0 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            Ok(())
        })?;
        self.get(item_id)
    }

    pub fn attach_to_task(&self, item_id: &str, task_id: &str) -> Result<(), CaptureError> {
        let item = self.get(item_id)?;
        if let Some(reason) = item.quarantine_reason {
            return Err(CaptureError::Quarantined(reason));
        }
        let timestamp = now()?;
        self.storage.with_connection(|connection| {
            connection.execute(
                "INSERT OR IGNORE INTO context_links (context_item_id, entity_kind, entity_id, created_at)
                 VALUES (?1, 'task', ?2, ?3)",
                params![item_id, task_id, timestamp],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    pub fn search(
        &self,
        repository_id: &str,
        task_id: Option<&str>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ContextItem>, CaptureError> {
        let pattern = format!("%{}%", query.trim().to_lowercase());
        let rows: Vec<String> = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT DISTINCT c.id FROM context_items c
                 LEFT JOIN context_links l ON l.context_item_id = c.id AND l.entity_kind = 'task'
                 WHERE c.repository_id = ?1
                   AND (?2 IS NULL OR l.entity_id = ?2)
                   AND (?3 = '%%' OR lower(c.title) LIKE ?3 OR lower(COALESCE(c.body_text, '')) LIKE ?3)
                 ORDER BY c.captured_at DESC, c.id DESC LIMIT ?4",
            )?;
            statement
                .query_map(
                    params![repository_id, task_id, pattern, limit.clamp(1, 100) as i64],
                    |row| row.get(0),
                )?
                .collect()
        })?;
        rows.into_iter().map(|id| self.get(&id)).collect()
    }

    pub fn get(&self, item_id: &str) -> Result<ContextItem, CaptureError> {
        let row = self.storage.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT c.id, c.repository_id, c.kind, c.title, c.body_text, c.original_uri,
                            c.mime_type, c.content_hash, c.classification, c.quarantine_reason,
                            a.storage_key, ia.original_name, c.captured_at
                     FROM context_items c
                     LEFT JOIN context_item_artifacts ia ON ia.context_item_id = c.id
                     LEFT JOIN context_artifacts a ON a.digest = ia.artifact_digest
                     WHERE c.id = ?1",
                    [item_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, Option<String>>(4)?,
                            row.get::<_, Option<String>>(5)?,
                            row.get::<_, String>(6)?,
                            row.get::<_, String>(7)?,
                            row.get::<_, Option<String>>(8)?,
                            row.get::<_, Option<String>>(9)?,
                            row.get::<_, Option<String>>(10)?,
                            row.get::<_, Option<String>>(11)?,
                            row.get::<_, String>(12)?,
                        ))
                    },
                )
                .optional()
        })?;
        let Some((
            id,
            repository_id,
            kind,
            title,
            body_text,
            original_uri,
            mime_type,
            content_hash,
            classification,
            quarantine_reason,
            storage_key,
            original_name,
            captured_at,
        )) = row
        else {
            return Err(CaptureError::ItemNotFound(item_id.to_owned()));
        };
        let artifact_path = storage_key.map(|key| self.artifact_root.join(key));
        if let Some(path) = &artifact_path {
            verify(path, &content_hash)?;
        }
        Ok(ContextItem {
            id,
            repository_id,
            kind: ContextKind::parse(&kind)?,
            title,
            body_text,
            original_uri,
            mime_type,
            content_hash,
            classification,
            quarantine_reason,
            artifact_path,
            original_name,
            captured_at,
        })
    }

    fn insert(&self, input: NewContext<'_>) -> Result<ContextItem, CaptureError> {
        let NewContext {
            repository_id,
            task_id,
            kind,
            title,
            body_text,
            original_uri,
            mime_type,
            content_hash,
            quarantine_reason,
            artifact,
        } = input;
        let item_id = Uuid::now_v7().to_string();
        let timestamp = now()?;
        let quarantine_for_insert = quarantine_reason.clone();
        let stored_id = self.storage.with_transaction(|transaction| {
            if quarantine_reason.is_none()
                && let Some(task_id) = task_id
            {
                transaction.query_row("SELECT id FROM tasks WHERE id = ?1", [task_id], |row| {
                    row.get::<_, String>(0)
                })?;
            }
            if let Some((artifact, _)) = artifact {
                transaction.execute(
                    "INSERT OR IGNORE INTO context_artifacts
                     (digest, storage_key, byte_size, mime_type, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![artifact.digest, artifact.relative_key, artifact.byte_size as i64, mime_type, timestamp],
                )?;
            }
            transaction.execute(
                "INSERT OR IGNORE INTO context_items
                 (id, repository_id, kind, title, body_text, original_uri, mime_type, content_hash,
                  classification, quarantine_reason, captured_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
                params![
                    item_id,
                    repository_id,
                    kind.as_str(),
                    title.trim(),
                    body_text,
                    original_uri,
                    mime_type,
                    content_hash,
                    Option::<String>::None,
                    quarantine_for_insert,
                    timestamp,
                ],
            )?;
            let stored_id: String = transaction.query_row(
                "SELECT id FROM context_items WHERE repository_id = ?1 AND content_hash = ?2 AND kind = ?3",
                params![repository_id, content_hash, kind.as_str()],
                |row| row.get(0),
            )?;
            if let Some((artifact, original_name)) = artifact {
                transaction.execute(
                    "INSERT OR IGNORE INTO context_item_artifacts
                     (context_item_id, artifact_digest, original_name) VALUES (?1, ?2, ?3)",
                    params![stored_id, artifact.digest, original_name],
                )?;
            }
            transaction.execute(
                "INSERT OR IGNORE INTO context_fts (context_item_id, title, body_text)
                 SELECT ?1, ?2, ?3 WHERE NOT EXISTS
                 (SELECT 1 FROM context_fts WHERE context_item_id = ?1)",
                params![stored_id, title.trim(), body_text],
            )?;
            if quarantine_reason.is_none()
                && let Some(task_id) = task_id
            {
                transaction.execute(
                    "INSERT OR IGNORE INTO context_links
                     (context_item_id, entity_kind, entity_id, created_at)
                     VALUES (?1, 'task', ?2, ?3)",
                    params![stored_id, task_id, timestamp],
                )?;
            }
            Ok(stored_id)
        })?;
        self.get(&stored_id)
    }
}

fn scan_file_for_secret(source: &Path, mime_type: &str) -> Result<Option<String>, CaptureError> {
    if !mime_type.starts_with("text/") {
        return Ok(None);
    }
    let text = fs::read_to_string(source).map_err(|source_error| CaptureError::Filesystem {
        path: source.to_owned(),
        source: source_error,
    })?;
    Ok(detect_secret(&text))
}

fn detect_secret(text: &str) -> Option<String> {
    let upper = text.to_ascii_uppercase();
    if upper.contains("-----BEGIN") && upper.contains("PRIVATE KEY-----") {
        return Some("private key material detected".into());
    }
    if text
        .split(|character: char| !character.is_ascii_alphanumeric())
        .any(|word| {
            word.starts_with("AKIA")
                && word.len() == 20
                && word
                    .chars()
                    .all(|character| character.is_ascii_uppercase() || character.is_ascii_digit())
        })
    {
        return Some("AWS access key pattern detected".into());
    }
    if text.split_whitespace().any(|word| {
        (word.starts_with("sk-") && word.len() >= 20)
            || (word.starts_with("ghp_") && word.len() >= 20)
    }) {
        return Some("API token pattern detected".into());
    }
    for line in upper.lines() {
        if [
            "API_KEY",
            "ACCESS_TOKEN",
            "AUTH_TOKEN",
            "PASSWORD",
            "SECRET_KEY",
        ]
        .iter()
        .any(|name| line.contains(name))
            && (line.contains('=') || line.contains(':'))
        {
            return Some("credential assignment pattern detected".into());
        }
    }
    None
}

fn now() -> Result<String, time::error::Format> {
    OffsetDateTime::now_utc().format(&Rfc3339)
}
