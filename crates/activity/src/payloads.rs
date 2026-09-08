//! Content-addressed storage for activity payloads.
//!
//! Command output, diffs, and test results are the bulk of what a run produces
//! and the part most likely to repeat. Storing them by SHA-256 digest keeps the
//! event rows small enough to page through and makes an identical payload cost
//! nothing the second time it appears.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mars_storage::Storage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::ActivityError;

/// A payload that has been written to the store and can be referenced by digest.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredPayload {
    pub digest: String,
    pub media_type: String,
    pub byte_size: usize,
    /// True when this call created the object, false when an identical payload
    /// was already present.
    pub created: bool,
}

#[derive(Clone)]
pub struct PayloadStore {
    storage: Arc<Storage>,
    root: PathBuf,
}

impl PayloadStore {
    #[must_use]
    pub fn new(storage: Arc<Storage>, root: impl AsRef<Path>) -> Self {
        Self {
            storage,
            root: root.as_ref().to_owned(),
        }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Write `bytes` under its digest, returning the existing object untouched
    /// when the same content is already stored.
    pub fn put(&self, media_type: &str, bytes: &[u8]) -> Result<StoredPayload, ActivityError> {
        let digest = digest_of(bytes);
        let relative = relative_path(&digest);
        let absolute = self.root.join(&relative);

        let existing = self.storage.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT byte_size, media_type FROM activity_payloads WHERE digest = ?1",
                    [&digest],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
                )
                .map(Some)
                .or_else(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })
        })?;

        if let Some((byte_size, stored_media_type)) = existing {
            if absolute.is_file() {
                return Ok(StoredPayload {
                    digest,
                    media_type: stored_media_type,
                    byte_size: usize::try_from(byte_size).unwrap_or(bytes.len()),
                    created: false,
                });
            }
            // The row outlived its object; rewrite the bytes rather than hand
            // back a reference that cannot be read.
            self.write_object(&absolute, bytes)?;
            return Ok(StoredPayload {
                digest,
                media_type: stored_media_type,
                byte_size: bytes.len(),
                created: true,
            });
        }

        self.write_object(&absolute, bytes)?;
        let relative_key = relative.to_string_lossy().replace('\\', "/");
        self.storage.with_connection(|connection| {
            connection.execute(
                "INSERT INTO activity_payloads
                     (digest, byte_size, media_type, relative_path, created_at)
                 VALUES (?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                rusqlite::params![
                    &digest,
                    i64::try_from(bytes.len()).unwrap_or(i64::MAX),
                    media_type,
                    &relative_key,
                ],
            )?;
            Ok(())
        })?;

        Ok(StoredPayload {
            digest,
            media_type: media_type.to_owned(),
            byte_size: bytes.len(),
            created: true,
        })
    }

    /// Read a payload back by digest, verifying the bytes still hash to it.
    pub fn get(&self, digest: &str) -> Result<Vec<u8>, ActivityError> {
        let absolute = self.root.join(relative_path(digest));
        let bytes = fs::read(&absolute).map_err(|source| ActivityError::PayloadIo {
            path: absolute.clone(),
            source,
        })?;
        if digest_of(&bytes) != digest {
            return Err(ActivityError::PayloadCorrupt {
                digest: digest.to_owned(),
            });
        }
        Ok(bytes)
    }

    fn write_object(&self, absolute: &Path, bytes: &[u8]) -> Result<(), ActivityError> {
        if let Some(parent) = absolute.parent() {
            fs::create_dir_all(parent).map_err(|source| ActivityError::PayloadIo {
                path: parent.to_owned(),
                source,
            })?;
        }
        let staging = self.root.join("staging");
        fs::create_dir_all(&staging).map_err(|source| ActivityError::PayloadIo {
            path: staging.clone(),
            source,
        })?;
        let temporary = staging.join(Uuid::now_v7().to_string());
        fs::write(&temporary, bytes).map_err(|source| ActivityError::PayloadIo {
            path: temporary.clone(),
            source,
        })?;
        match fs::rename(&temporary, absolute) {
            Ok(()) => Ok(()),
            Err(error) if absolute.is_file() => {
                // Another writer won the race with byte-identical content.
                let _ = fs::remove_file(&temporary);
                if error.kind() == io::ErrorKind::PermissionDenied || absolute.is_file() {
                    Ok(())
                } else {
                    Err(ActivityError::PayloadIo {
                        path: absolute.to_owned(),
                        source: error,
                    })
                }
            }
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                Err(ActivityError::PayloadIo {
                    path: absolute.to_owned(),
                    source: error,
                })
            }
        }
    }
}

#[must_use]
pub fn digest_of(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut rendered = String::with_capacity(digest.len() * 2);
    for byte in digest {
        rendered.push_str(&format!("{byte:02x}"));
    }
    rendered
}

fn relative_path(digest: &str) -> PathBuf {
    let (prefix, rest) = digest.split_at(2.min(digest.len()));
    let (middle, tail) = rest.split_at(2.min(rest.len()));
    PathBuf::from("objects")
        .join(prefix)
        .join(middle)
        .join(tail)
}
