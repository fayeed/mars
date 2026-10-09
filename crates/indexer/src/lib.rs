//! Repository indexing: files, symbols, graph edges, chunks, and Git history.
//!
//! Every layer records the Git blob SHA it was derived from, so a consumer can
//! always ask whether an answer is still true of the working tree.

pub mod api;
pub mod chunking;
pub mod embedding;
pub mod facts;
pub mod golang;
pub mod history;
pub mod javascript;
pub mod languages;
pub mod parse;
pub mod publish;
pub mod python;
pub mod rustlang;
pub mod search;
pub mod structured;
pub mod walk;
pub mod watcher;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mars_storage::Storage;
use mars_storage::projects::ProjectRepository;
use rusqlite::params;
use serde::Serialize;
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use chunking::chunk_file;
use publish::{PreparedChunk, PreparedFile, PublishInput, Rename};
use walk::SourceFile;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexSummary {
    pub run_id: String,
    pub mode: String,
    pub indexed_files: usize,
    pub skipped_files: usize,
    pub deleted_files: usize,
    pub renamed_files: usize,
    pub chunk_count: usize,
    pub ignored_paths: usize,
    pub commits_ingested: usize,
    pub head_oid: Option<String>,
}

/// What a run covers.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum IndexMode {
    /// Walk the whole working tree.
    #[default]
    Full,
    /// Revisit only the paths named by the caller.
    Paths,
}

impl IndexMode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Paths => "paths",
        }
    }
}

/// Progress a caller can stream while a run is in flight.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "event"
)]
pub enum IndexEvent {
    Started {
        run_id: String,
        mode: String,
        total_files: usize,
    },
    FileIndexed {
        run_id: String,
        relative_path: String,
        position: usize,
        total_files: usize,
    },
    Phase {
        run_id: String,
        phase: String,
    },
    Completed {
        run_id: String,
        summary: IndexSummary,
    },
    Failed {
        run_id: String,
        error: String,
    },
    Cancelled {
        run_id: String,
    },
}

#[derive(Clone, Debug)]
pub struct IndexOptions {
    /// Restrict the run to these repository-relative paths.
    pub paths: Option<Vec<String>>,
    /// How far back a cold history read reaches.
    pub history_depth: usize,
    /// Read Git history as part of the run.
    pub include_history: bool,
}

impl Default for IndexOptions {
    fn default() -> Self {
        Self {
            paths: None,
            history_depth: history::DEFAULT_HISTORY_DEPTH,
            include_history: true,
        }
    }
}

impl IndexOptions {
    #[must_use]
    pub fn for_paths(paths: Vec<String>) -> Self {
        Self {
            paths: Some(paths),
            include_history: true,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn mode(&self) -> IndexMode {
        if self.paths.is_some() {
            IndexMode::Paths
        } else {
            IndexMode::Full
        }
    }
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
    #[error("Git command failed: {0}")]
    Git(String),
    #[error("file watching failed: {0}")]
    Watch(String),
    #[error("index run `{run_id}` was cancelled")]
    Cancelled { run_id: String },
}

/// Cloneable so a caller can move one onto a worker thread.
///
/// Both fields are handles to shared state rather than the state itself, so a
/// clone is a pointer copy and every clone reads and writes the same database.
#[derive(Clone)]
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

    /// Indexes the whole repository.
    pub fn index_repository(
        &self,
        repository_id: &str,
        cancellation: &CancellationToken,
    ) -> Result<IndexSummary, IndexError> {
        self.run(
            repository_id,
            &IndexOptions::default(),
            cancellation,
            |_| {},
        )
    }

    /// Indexes the whole repository, reporting each file as it is visited.
    pub fn index_repository_observing(
        &self,
        repository_id: &str,
        cancellation: &CancellationToken,
        mut observer: impl FnMut(usize, &Path),
    ) -> Result<IndexSummary, IndexError> {
        self.run(
            repository_id,
            &IndexOptions::default(),
            cancellation,
            |event| {
                if let IndexEvent::FileIndexed {
                    relative_path,
                    position,
                    ..
                } = event
                {
                    observer(*position, Path::new(relative_path));
                }
            },
        )
    }

    /// Re-indexes the named paths only.
    pub fn reindex_paths(
        &self,
        repository_id: &str,
        paths: Vec<String>,
        cancellation: &CancellationToken,
    ) -> Result<IndexSummary, IndexError> {
        self.run(
            repository_id,
            &IndexOptions::for_paths(paths),
            cancellation,
            |_| {},
        )
    }

    /// Runs an index pass, streaming progress to `observer`.
    pub fn run(
        &self,
        repository_id: &str,
        options: &IndexOptions,
        cancellation: &CancellationToken,
        mut observer: impl FnMut(&IndexEvent),
    ) -> Result<IndexSummary, IndexError> {
        let repository = self.projects.find_repository(repository_id)?;
        let run_id = Uuid::now_v7().to_string();
        let mode = options.mode();
        let started_at = timestamp()?;
        let head_oid = history::head_oid(&repository.root_path);

        self.storage.with_connection(|connection| {
            connection.execute(
                "INSERT INTO index_runs
                 (id, repository_id, status, started_at, mode, base_head_oid)
                 VALUES (?1, ?2, 'running', ?3, ?4, ?5)",
                params![
                    run_id,
                    repository_id,
                    started_at,
                    mode.as_str(),
                    head_oid.as_deref()
                ],
            )?;
            Ok(())
        })?;

        observer(&IndexEvent::Started {
            run_id: run_id.clone(),
            mode: mode.as_str().to_owned(),
            total_files: 0,
        });

        let result = self.prepare(
            repository_id,
            &repository.root_path,
            &run_id,
            options,
            head_oid.as_deref(),
            cancellation,
            &mut observer,
        );
        match result {
            Ok(summary) => {
                observer(&IndexEvent::Completed {
                    run_id: run_id.clone(),
                    summary: summary.clone(),
                });
                Ok(summary)
            }
            Err(IndexError::Cancelled { .. }) => {
                self.finish_run(&run_id, "interrupted", None)?;
                observer(&IndexEvent::Cancelled {
                    run_id: run_id.clone(),
                });
                Err(IndexError::Cancelled { run_id })
            }
            Err(error) => {
                self.finish_run(&run_id, "failed", Some(&error.to_string()))?;
                observer(&IndexEvent::Failed {
                    run_id: run_id.clone(),
                    error: error.to_string(),
                });
                Err(error)
            }
        }
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn prepare(
        &self,
        repository_id: &str,
        root: &Path,
        run_id: &str,
        options: &IndexOptions,
        head_oid: Option<&str>,
        cancellation: &CancellationToken,
        observer: &mut impl FnMut(&IndexEvent),
    ) -> Result<IndexSummary, IndexError> {
        let existing = self.existing_files(repository_id, options.paths.as_deref())?;

        // A targeted run still consults the ignore rules, so a path that has just
        // become ignored is retired rather than re-indexed.
        let (sources, ignored, covered) = match options.paths.as_deref() {
            Some(paths) => {
                let allowed = walk::included_among(root, paths)?;
                let requested: Vec<String> = allowed.into_iter().collect();
                let covered: HashSet<String> =
                    paths.iter().map(|path| path.replace('\\', "/")).collect();
                (walk::read_paths(root, &requested)?, None, Some(covered))
            }
            None => {
                let outcome = walk::walk(root)?;
                (outcome.files, Some(outcome.ignored), None)
            }
        };

        let seen: HashSet<String> = sources
            .iter()
            .map(|source| source.relative_path.clone())
            .collect();
        // In a targeted run only the requested paths may be retired; in a full
        // run anything no longer on disk is.
        let deleted: Vec<String> = existing
            .keys()
            .filter(|path| !seen.contains(*path))
            .filter(|path| {
                covered
                    .as_ref()
                    .is_none_or(|covered| covered.contains(*path))
            })
            .cloned()
            .collect();

        let (renames, deleted) = detect_renames(&existing, &sources, &deleted);
        let renamed_targets: HashSet<&str> =
            renames.iter().map(|rename| rename.to.as_str()).collect();

        let total_files = sources.len();
        observer(&IndexEvent::Started {
            run_id: run_id.into(),
            mode: options.mode().as_str().to_owned(),
            total_files,
        });

        let mut prepared: Vec<PreparedFile> = Vec::new();
        let mut skipped = 0;
        for (position, source) in sources.into_iter().enumerate() {
            observer(&IndexEvent::FileIndexed {
                run_id: run_id.into(),
                relative_path: source.relative_path.clone(),
                position: position + 1,
                total_files,
            });
            if cancellation.is_cancelled() {
                return Err(IndexError::Cancelled {
                    run_id: run_id.into(),
                });
            }
            // A renamed file is republished so its chunk headers and evidence
            // carry the new path, even though its bytes are unchanged.
            let unchanged = existing
                .get(&source.relative_path)
                .is_some_and(|existing| existing.content_hash == source.content_hash)
                && !renamed_targets.contains(source.relative_path.as_str());
            if unchanged {
                skipped += 1;
                continue;
            }
            prepared.push(prepare_file(repository_id, source)?);
        }

        if cancellation.is_cancelled() {
            return Err(IndexError::Cancelled {
                run_id: run_id.into(),
            });
        }

        let branch = history::current_branch(root);
        let mut commits_ingested = 0;
        let mut history_head = None;
        if options.include_history {
            observer(&IndexEvent::Phase {
                run_id: run_id.into(),
                phase: "history".into(),
            });
            let since = self.history_cursor(repository_id)?;
            let summary = history::ingest(
                &self.storage,
                repository_id,
                root,
                since.as_deref(),
                options.history_depth,
            )?;
            commits_ingested = summary.commits_ingested;
            history_head = summary.head_oid;
        }

        if cancellation.is_cancelled() {
            return Err(IndexError::Cancelled {
                run_id: run_id.into(),
            });
        }

        observer(&IndexEvent::Phase {
            run_id: run_id.into(),
            phase: "publish".into(),
        });
        let chunk_count = prepared.iter().map(|file| file.chunks.len()).sum();
        let ignored_count = ignored.as_ref().map_or(0, Vec::len);
        publish::publish(
            &self.storage,
            &PublishInput {
                repository_id,
                run_id,
                files: &prepared,
                deleted: &deleted,
                renames: &renames,
                ignored: ignored.as_deref(),
                files_seen: seen.len(),
                skipped_files: skipped,
                head_oid,
                branch: branch.as_deref(),
                history_head_oid: history_head.as_deref(),
            },
        )?;

        Ok(IndexSummary {
            run_id: run_id.into(),
            mode: options.mode().as_str().to_owned(),
            indexed_files: prepared.len(),
            skipped_files: skipped,
            deleted_files: deleted.len(),
            renamed_files: renames.len(),
            chunk_count,
            ignored_paths: ignored_count,
            commits_ingested,
            head_oid: head_oid.map(str::to_owned),
        })
    }

    /// Loads the indexed state a run diffs against: every active file for a
    /// full run, or only the covered paths for a targeted one.
    fn existing_files(
        &self,
        repository_id: &str,
        paths: Option<&[String]>,
    ) -> Result<HashMap<String, ExistingFile>, IndexError> {
        let read = |row: &rusqlite::Row<'_>| {
            Ok((
                row.get::<_, String>(0)?,
                ExistingFile {
                    content_hash: row.get(1)?,
                    blob_sha: row.get(2)?,
                },
            ))
        };
        Ok(self.storage.with_connection(|connection| match paths {
            Some(paths) => {
                let mut statement = connection.prepare_cached(
                    "SELECT relative_path, content_hash, IFNULL(blob_sha, '')
                     FROM files
                     WHERE repository_id = ?1 AND relative_path = ?2 AND deleted_at IS NULL",
                )?;
                let mut existing = HashMap::new();
                for path in paths {
                    let path = path.replace('\\', "/");
                    let mut rows = statement.query_map(params![repository_id, path], read)?;
                    if let Some(row) = rows.next().transpose()? {
                        existing.insert(row.0, row.1);
                    }
                }
                Ok(existing)
            }
            None => {
                let mut statement = connection.prepare(
                    "SELECT relative_path, content_hash, IFNULL(blob_sha, '')
                     FROM files WHERE repository_id = ?1 AND deleted_at IS NULL",
                )?;
                statement.query_map([repository_id], read)?.collect()
            }
        })?)
    }

    fn history_cursor(&self, repository_id: &str) -> Result<Option<String>, IndexError> {
        Ok(self.storage.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT history_head_oid FROM index_state WHERE repository_id = ?1",
                    [repository_id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .or_else(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    error => Err(error),
                })
        })?)
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

#[derive(Clone, Debug)]
struct ExistingFile {
    content_hash: String,
    blob_sha: String,
}

fn prepare_file(repository_id: &str, source: SourceFile) -> Result<PreparedFile, IndexError> {
    let parsed = parse::parse(source)?;
    let chunks = chunk_file(repository_id, &parsed)
        .into_iter()
        .map(|fact| PreparedChunk {
            vector: embedding::embed(&fact.embedded_text()),
            fact,
        })
        .collect();
    Ok(PreparedFile { parsed, chunks })
}

/// Pairs a disappeared path with an appeared path holding the same Git blob.
/// Content identity is what makes this a move rather than a delete plus an add,
/// and it keeps the file's identity — and every symbol id under it — stable.
fn detect_renames(
    existing: &HashMap<String, ExistingFile>,
    sources: &[SourceFile],
    deleted: &[String],
) -> (Vec<Rename>, Vec<String>) {
    let mut by_blob: HashMap<&str, Vec<&str>> = HashMap::new();
    for path in deleted {
        if let Some(file) = existing.get(path)
            && !file.blob_sha.is_empty()
        {
            by_blob.entry(&file.blob_sha).or_default().push(path);
        }
    }
    if by_blob.is_empty() {
        return (Vec::new(), deleted.to_vec());
    }

    let mut renames = Vec::new();
    let mut matched: HashSet<&str> = HashSet::new();
    for source in sources {
        if existing.contains_key(&source.relative_path) {
            continue;
        }
        let Some(candidates) = by_blob.get_mut(source.blob_sha.as_str()) else {
            continue;
        };
        // An ambiguous blob (several identical files moved at once) is left to
        // the plain delete-and-add path rather than guessed at.
        let Some(from) = candidates.pop() else {
            continue;
        };
        if !candidates.is_empty() {
            continue;
        }
        matched.insert(from);
        renames.push(Rename {
            from: from.to_owned(),
            to: source.relative_path.clone(),
        });
    }

    let remaining = deleted
        .iter()
        .filter(|path| !matched.contains(path.as_str()))
        .cloned()
        .collect();
    renames.sort_by(|left, right| left.from.cmp(&right.from));
    (renames, remaining)
}

pub(crate) fn timestamp() -> Result<String, IndexError> {
    Ok(OffsetDateTime::now_utc().format(&Rfc3339)?)
}
