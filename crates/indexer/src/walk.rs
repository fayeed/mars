use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use ignore::WalkBuilder;
use ignore::gitignore::GitignoreBuilder;
use mars_security::{is_default_ignored_directory, is_sensitive_filename};
use sha1::Sha1;
use sha2::{Digest, Sha256};

use crate::IndexError;
use crate::languages::{self, Language};

/// Files larger than this are recorded in the file layer but never parsed or chunked.
pub const MAX_PARSED_BYTES: u64 = 1_048_576;

/// A file that belongs in the index, with the Git identity it was derived from.
#[derive(Clone, Debug)]
pub struct SourceFile {
    pub relative_path: String,
    pub language: String,
    pub content: Vec<u8>,
    pub content_hash: String,
    pub blob_sha: String,
    pub byte_size: u64,
    pub mtime_ns: Option<i64>,
    pub is_binary: bool,
    pub parseable: bool,
}

impl SourceFile {
    #[must_use]
    pub fn parsed_language(&self) -> Option<Language> {
        languages::from_name(&self.language)
    }

    #[must_use]
    pub fn text(&self) -> Option<&str> {
        std::str::from_utf8(&self.content).ok()
    }
}

/// Why the walker refused to index a path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IgnoreReason {
    Gitignore,
    Marsignore,
    MarsDefaultDirectory,
    SensitiveFile,
}

impl IgnoreReason {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gitignore => "gitignore",
            Self::Marsignore => "marsignore",
            Self::MarsDefaultDirectory => "mars-default-directory",
            Self::SensitiveFile => "sensitive-file",
        }
    }
}

#[derive(Clone, Debug)]
pub struct IgnoredPath {
    pub relative_path: String,
    pub is_directory: bool,
    pub reason: IgnoreReason,
}

#[derive(Debug, Default)]
pub struct WalkOutcome {
    pub files: Vec<SourceFile>,
    pub ignored: Vec<IgnoredPath>,
}

/// Walks the working tree for indexable files, then walks it again without ignore
/// filtering to record which entries were excluded and why.
pub fn walk(root: &Path) -> Result<WalkOutcome, IndexError> {
    let included = included_paths(root)?;
    let ignored = ignored_paths(root, &included)?;
    let mut files = Vec::with_capacity(included.len());
    for relative_path in &included {
        files.push(read_source(root, relative_path)?);
    }
    Ok(WalkOutcome { files, ignored })
}

/// Lists the indexable paths without reading or hashing their contents.
pub fn source_paths(root: &Path) -> Result<Vec<String>, IndexError> {
    Ok(included_paths(root)?.into_iter().collect())
}

/// Reads a specific set of paths, skipping any that no longer exist.
pub fn read_paths(root: &Path, relative_paths: &[String]) -> Result<Vec<SourceFile>, IndexError> {
    let mut files = Vec::new();
    for relative_path in relative_paths {
        if !root.join(relative_path).is_file() {
            continue;
        }
        files.push(read_source(root, relative_path)?);
    }
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(files)
}

fn included_paths(root: &Path) -> Result<BTreeSet<String>, IndexError> {
    let mut builder = WalkBuilder::new(root);
    builder.add_custom_ignore_filename(".marsignore");
    builder.hidden(false);
    builder.filter_entry(|entry| !is_mars_excluded(entry.path()));

    let mut paths = BTreeSet::new();
    for result in builder.build() {
        let entry = result.map_err(|error| IndexError::Walk(error.to_string()))?;
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        paths.insert(relative_path(root, entry.path())?);
    }
    Ok(paths)
}

/// Walks with every standard filter disabled so excluded entries stay visible.
/// Mars-excluded directories are recorded once instead of being descended into.
fn ignored_paths(root: &Path, included: &BTreeSet<String>) -> Result<Vec<IgnoredPath>, IndexError> {
    let marsignore = marsignore_matcher(root)?;
    let mut builder = WalkBuilder::new(root);
    builder.standard_filters(false);
    builder.hidden(false);
    builder.filter_entry(|entry| entry.depth() == 0 || !is_mars_excluded_directory(entry.path()));

    let mut ignored = Vec::new();
    for result in builder.build() {
        let entry = result.map_err(|error| IndexError::Walk(error.to_string()))?;
        if entry.depth() == 0 {
            continue;
        }
        let path = entry.path();
        let relative = relative_path(root, path)?;
        if entry.file_type().is_some_and(|kind| kind.is_dir()) {
            continue;
        }
        if included.contains(&relative) {
            continue;
        }
        ignored.push(IgnoredPath {
            relative_path: relative.clone(),
            is_directory: false,
            reason: file_ignore_reason(path, &relative, marsignore.as_ref()),
        });
    }

    for directory in excluded_directories(root)? {
        ignored.push(IgnoredPath {
            relative_path: directory,
            is_directory: true,
            reason: IgnoreReason::MarsDefaultDirectory,
        });
    }
    ignored.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(ignored)
}

/// Finds the Mars-excluded directories at the boundary of the walk, so a pruned
/// tree such as `node_modules/` is recorded once rather than per contained file.
fn excluded_directories(root: &Path) -> Result<Vec<String>, IndexError> {
    let mut builder = WalkBuilder::new(root);
    builder.standard_filters(false);
    builder.hidden(false);
    builder.filter_entry(|entry| entry.depth() == 0 || !is_mars_excluded_directory(entry.path()));

    let mut directories = Vec::new();
    for result in builder.build() {
        let entry = result.map_err(|error| IndexError::Walk(error.to_string()))?;
        if !entry.file_type().is_some_and(|kind| kind.is_dir()) {
            continue;
        }
        let listing = match fs::read_dir(entry.path()) {
            Ok(listing) => listing,
            Err(_) => continue,
        };
        for child in listing {
            let child = child.map_err(|source| IndexError::Filesystem {
                path: entry.path().into(),
                source,
            })?;
            if child.path().is_dir() && is_mars_excluded_directory(&child.path()) {
                directories.push(relative_path(root, &child.path())?);
            }
        }
    }
    Ok(directories)
}

fn file_ignore_reason(
    path: &Path,
    relative: &str,
    marsignore: Option<&ignore::gitignore::Gitignore>,
) -> IgnoreReason {
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
    if is_sensitive_filename(name) {
        return IgnoreReason::SensitiveFile;
    }
    let matched = marsignore.is_some_and(|matcher| {
        matcher
            .matched_path_or_any_parents(Path::new(relative), false)
            .is_ignore()
    });
    if matched {
        IgnoreReason::Marsignore
    } else {
        IgnoreReason::Gitignore
    }
}

/// Builds a matcher from every `.marsignore` in the tree so an excluded file can
/// be attributed to the Mars-level list rather than to Git.
fn marsignore_matcher(root: &Path) -> Result<Option<ignore::gitignore::Gitignore>, IndexError> {
    let mut builder = GitignoreBuilder::new("");
    let mut found = false;
    let mut walker = WalkBuilder::new(root);
    walker.standard_filters(false);
    walker.hidden(false);
    walker.filter_entry(|entry| entry.depth() == 0 || !is_mars_excluded_directory(entry.path()));
    for result in walker.build() {
        let entry = result.map_err(|error| IndexError::Walk(error.to_string()))?;
        if entry.file_name() != ".marsignore" {
            continue;
        }
        let prefix = relative_path(root, entry.path())?
            .strip_suffix(".marsignore")
            .unwrap_or_default()
            .to_owned();
        let contents = fs::read_to_string(entry.path()).map_err(|source| IndexError::Filesystem {
            path: entry.path().into(),
            source,
        })?;
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let pattern = format!("{prefix}{line}");
            builder
                .add_line(None, &pattern)
                .map_err(|error| IndexError::Walk(error.to_string()))?;
            found = true;
        }
    }
    if !found {
        return Ok(None);
    }
    builder
        .build()
        .map(Some)
        .map_err(|error| IndexError::Walk(error.to_string()))
}

fn is_mars_excluded(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| is_default_ignored_directory(name) || is_sensitive_filename(name))
}

fn is_mars_excluded_directory(path: &Path) -> bool {
    path.is_dir()
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(is_default_ignored_directory)
}

fn read_source(root: &Path, relative_path: &str) -> Result<SourceFile, IndexError> {
    let absolute = root.join(relative_path);
    let metadata = fs::metadata(&absolute).map_err(|source| IndexError::Filesystem {
        path: absolute.clone(),
        source,
    })?;
    let byte_size = metadata.len();
    let mtime_ns = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_nanos()).ok());

    let digests = digest_file(&absolute, byte_size)?;
    let content = if byte_size <= MAX_PARSED_BYTES && !digests.is_binary {
        fs::read(&absolute).map_err(|source| IndexError::Filesystem {
            path: absolute.clone(),
            source,
        })?
    } else {
        Vec::new()
    };
    let language = languages::detect(Path::new(relative_path));
    let parseable = !digests.is_binary
        && byte_size <= MAX_PARSED_BYTES
        && std::str::from_utf8(&content).is_ok()
        && language.is_some();

    Ok(SourceFile {
        relative_path: relative_path.to_owned(),
        language: language.map_or_else(
            || {
                if digests.is_binary {
                    "binary".to_owned()
                } else {
                    "unknown".to_owned()
                }
            },
            |language| language.name().to_owned(),
        ),
        content_hash: digests.sha256,
        blob_sha: digests.blob_sha,
        byte_size,
        mtime_ns,
        is_binary: digests.is_binary,
        parseable,
        content,
    })
}

/// Computes the Git blob SHA-1 of a file on disk. This is the identity the
/// freshness contract compares against.
pub fn blob_sha(path: &Path) -> Result<String, IndexError> {
    let metadata = fs::metadata(path).map_err(|source| IndexError::Filesystem {
        path: path.into(),
        source,
    })?;
    Ok(digest_file(path, metadata.len())?.blob_sha)
}

struct FileDigests {
    sha256: String,
    blob_sha: String,
    is_binary: bool,
}

/// Computes the SHA-256 content hash and the Git blob SHA-1 in a single streaming
/// pass, so a large file never has to be held in memory to get its identity.
fn digest_file(path: &Path, byte_size: u64) -> Result<FileDigests, IndexError> {
    let mut file = fs::File::open(path).map_err(|source| IndexError::Filesystem {
        path: path.into(),
        source,
    })?;
    let mut content = Sha256::new();
    let mut blob = Sha1::new();
    blob.update(format!("blob {byte_size}\0").as_bytes());

    let mut buffer = vec![0_u8; 64 * 1024];
    let mut is_binary = false;
    let mut inspected = 0_usize;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| IndexError::Filesystem {
                path: path.into(),
                source,
            })?;
        if read == 0 {
            break;
        }
        let block = &buffer[..read];
        content.update(block);
        blob.update(block);
        if inspected < 8192 {
            is_binary = is_binary || block.contains(&0);
            inspected += read;
        }
    }
    Ok(FileDigests {
        sha256: hex(&content.finalize()),
        blob_sha: hex(&blob.finalize()),
        is_binary,
    })
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

pub(crate) fn relative_path(root: &Path, path: &Path) -> Result<String, IndexError> {
    Ok(path
        .strip_prefix(root)
        .map_err(|_| IndexError::OutsideRepository(PathBuf::from(path)))?
        .to_string_lossy()
        .replace('\\', "/"))
}
