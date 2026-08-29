use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::capture::CaptureError;

pub(crate) const MAX_ARTIFACT_BYTES: usize = 25 * 1024 * 1024;

pub(crate) struct StoredArtifact {
    pub digest: String,
    pub path: PathBuf,
    pub relative_key: String,
    pub byte_size: usize,
    pub created: bool,
}

pub(crate) fn store(source: &Path, root: &Path) -> Result<StoredArtifact, CaptureError> {
    let metadata = fs::metadata(source).map_err(|source_error| CaptureError::Filesystem {
        path: source.to_owned(),
        source: source_error,
    })?;
    if !metadata.is_file() {
        return Err(CaptureError::NotAFile(source.to_owned()));
    }
    let byte_size = usize::try_from(metadata.len()).unwrap_or(usize::MAX);
    if byte_size > MAX_ARTIFACT_BYTES {
        return Err(CaptureError::ArtifactTooLarge {
            bytes: byte_size,
            maximum: MAX_ARTIFACT_BYTES,
        });
    }
    let bytes = fs::read(source).map_err(|source_error| CaptureError::Filesystem {
        path: source.to_owned(),
        source: source_error,
    })?;
    store_bytes(&bytes, root)
}

pub(crate) fn store_bytes(bytes: &[u8], root: &Path) -> Result<StoredArtifact, CaptureError> {
    if bytes.len() > MAX_ARTIFACT_BYTES {
        return Err(CaptureError::ArtifactTooLarge {
            bytes: bytes.len(),
            maximum: MAX_ARTIFACT_BYTES,
        });
    }
    let digest = encode_digest(bytes);
    let relative_key = format!("sha256/{}/{digest}", &digest[..2]);
    let path = root.join(&relative_key);
    if path.is_file() {
        verify(&path, &digest)?;
        return Ok(StoredArtifact {
            digest,
            path,
            relative_key,
            byte_size: bytes.len(),
            created: false,
        });
    }
    let parent = path
        .parent()
        .expect("content-addressed paths have a parent");
    fs::create_dir_all(parent).map_err(|source_error| CaptureError::Filesystem {
        path: parent.to_owned(),
        source: source_error,
    })?;
    let temporary = parent.join(format!(".capture-{}.tmp", Uuid::now_v7()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|source_error| CaptureError::Filesystem {
                path: temporary.clone(),
                source: source_error,
            })?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|source_error| CaptureError::Filesystem {
                path: temporary.clone(),
                source: source_error,
            })?;
        verify(&temporary, &digest)?;
        match fs::rename(&temporary, &path) {
            Ok(()) => Ok(true),
            Err(_) if path.is_file() => {
                let _ = fs::remove_file(&temporary);
                verify(&path, &digest)?;
                Ok(false)
            }
            Err(source_error) => Err(CaptureError::Filesystem {
                path: path.clone(),
                source: source_error,
            }),
        }
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    Ok(StoredArtifact {
        digest,
        path,
        relative_key,
        byte_size: bytes.len(),
        created: result?,
    })
}

pub(crate) fn verify(path: &Path, expected: &str) -> Result<(), CaptureError> {
    let bytes = fs::read(path).map_err(|source_error| CaptureError::Filesystem {
        path: path.to_owned(),
        source: source_error,
    })?;
    let actual = encode_digest(&bytes);
    if actual == expected {
        Ok(())
    } else {
        Err(CaptureError::DigestMismatch {
            path: path.to_owned(),
            expected: expected.to_owned(),
            actual,
        })
    }
}

pub(crate) fn encode_digest(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}
