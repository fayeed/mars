use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

use ignore::WalkBuilder;
use sha2::{Digest, Sha256};

use crate::IndexError;

const IGNORED_DIRECTORIES: [&str; 5] = [".git", "node_modules", "target", "dist", "build"];

#[derive(Clone, Debug)]
pub struct SourceFile {
    pub relative_path: String,
    pub language: String,
    pub content: Vec<u8>,
    pub content_hash: String,
    pub byte_size: u64,
    pub mtime_ns: Option<i64>,
}

pub fn source_files(root: &Path) -> Result<Vec<SourceFile>, IndexError> {
    let mut builder = WalkBuilder::new(root);
    builder.add_custom_ignore_filename(".marsignore");
    builder.hidden(false);
    builder.filter_entry(|entry| {
        entry
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .is_none_or(|name| !IGNORED_DIRECTORIES.contains(&name) && !is_sensitive(name))
    });

    let mut sources = Vec::new();
    for result in builder.build() {
        let entry = result.map_err(|error| IndexError::Walk(error.to_string()))?;
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let Some(language) = language_for(entry.path()) else {
            continue;
        };
        let metadata = entry
            .metadata()
            .map_err(|error| IndexError::Walk(error.to_string()))?;
        let content = fs::read(entry.path()).map_err(|source| IndexError::Filesystem {
            path: entry.path().into(),
            source,
        })?;
        if content.contains(&0) || std::str::from_utf8(&content).is_err() {
            continue;
        }
        let relative_path = entry
            .path()
            .strip_prefix(root)
            .map_err(|_| IndexError::OutsideRepository(entry.path().into()))?
            .to_string_lossy()
            .replace('\\', "/");
        let mtime_ns = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .and_then(|duration| i64::try_from(duration.as_nanos()).ok());
        sources.push(SourceFile {
            relative_path,
            language: language.into(),
            content_hash: format!("{:x}", Sha256::digest(&content)),
            byte_size: metadata.len(),
            mtime_ns,
            content,
        });
    }
    sources.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(sources)
}

fn language_for(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "ts" => Some("typescript"),
        "tsx" => Some("tsx"),
        "js" | "mjs" | "cjs" => Some("javascript"),
        "jsx" => Some("jsx"),
        _ => None,
    }
}

fn is_sensitive(name: &str) -> bool {
    ((name == ".env" || name.starts_with(".env.")) && name != ".env.example")
        || matches!(name, "id_rsa" | "id_ed25519" | "credentials")
        || name.ends_with(".pem")
        || name.ends_with(".key")
}
