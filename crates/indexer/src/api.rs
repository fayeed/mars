//! The internal, typed index API.
//!
//! Every answer that rests on stored rows carries the blob SHA it was derived
//! from and a freshness verdict, so a consumer is never handed a stale result
//! that looks current.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mars_storage::Storage;
use mars_storage::projects::ProjectRepository;
use rusqlite::{OptionalExtension, ToSql, params, params_from_iter};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::history::{BlameRange, FileChurn};
use crate::search::{SearchFilters, SearchHit, SearchService};
use crate::{
    IndexError, IndexEvent, IndexOptions, IndexSummary, Indexer, embedding, history, walk,
};

/// Traversal bounds. Answers stay explainable and fast by refusing to walk the
/// whole graph.
pub const MAX_GRAPH_DEPTH: usize = 5;
pub const MAX_GRAPH_RESULTS: usize = 500;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FreshnessState {
    /// The stored rows match the file on disk.
    Fresh,
    /// The file has changed since it was indexed.
    Stale,
    /// The file is gone from the working tree.
    Missing,
    /// The file was never indexed, so there is nothing to compare.
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Freshness {
    pub state: FreshnessState,
    pub relative_path: String,
    pub indexed_blob_sha: Option<String>,
    pub current_blob_sha: Option<String>,
}

impl Freshness {
    #[must_use]
    pub fn is_fresh(&self) -> bool {
        self.state == FreshnessState::Fresh
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexCounts {
    pub files: usize,
    pub symbols: usize,
    pub edges: usize,
    pub chunks: usize,
    pub commits: usize,
    pub ignored_paths: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexRunStatus {
    pub run_id: String,
    pub status: String,
    pub mode: String,
    pub files_seen: usize,
    pub indexed_files: usize,
    pub skipped_files: usize,
    pub deleted_files: usize,
    pub renamed_files: usize,
    pub chunk_count: usize,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStatus {
    pub repository_id: String,
    pub root_path: PathBuf,
    pub branch: Option<String>,
    pub head_oid: Option<String>,
    pub indexed_head_oid: Option<String>,
    /// False when the working tree has moved to a different commit since the
    /// last completed run, which is the usual cause of a stale index.
    pub head_matches_index: bool,
    pub embedding_model: String,
    pub embedding_dimensions: usize,
    pub counts: IndexCounts,
    pub last_run: Option<IndexRunStatus>,
    pub running_runs: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolLocation {
    pub symbol_id: String,
    pub stable_key: String,
    pub name: String,
    pub kind: String,
    pub qualified_name: Option<String>,
    pub relative_path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub start_byte: usize,
    pub end_byte: usize,
    pub signature: Option<String>,
    pub documentation: Option<String>,
    pub exported: bool,
    pub parent_symbol_id: Option<String>,
    pub blob_sha: Option<String>,
    pub freshness: Freshness,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    /// Edges leaving the symbol: what it depends on.
    Outgoing,
    /// Edges arriving at the symbol: who depends on it.
    Incoming,
    #[default]
    Both,
}

#[derive(Clone, Debug)]
pub struct NeighborOptions {
    pub kinds: Vec<String>,
    pub depth: usize,
    pub direction: Direction,
    pub limit: usize,
}

impl Default for NeighborOptions {
    fn default() -> Self {
        Self {
            kinds: Vec::new(),
            depth: 1,
            direction: Direction::Both,
            limit: 100,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    pub symbol_id: String,
    pub name: String,
    pub kind: String,
    pub relative_path: String,
    pub start_line: usize,
    pub end_line: usize,
    /// Hops from the queried symbol.
    pub distance: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdge {
    pub edge_kind: String,
    pub source_id: String,
    pub source_kind: String,
    pub target_id: Option<String>,
    pub target_key: String,
    pub confidence: f64,
    pub evidence_json: String,
    pub direction: Direction,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Neighborhood {
    pub symbol_id: String,
    pub depth: usize,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub truncated: bool,
    pub freshness: Freshness,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDependencies {
    pub relative_path: String,
    pub imports: Vec<String>,
    pub imported_by: Vec<String>,
    pub freshness: Freshness,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlameResult {
    pub relative_path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub ranges: Vec<BlameRange>,
    pub churn: FileChurn,
    /// Blame is read from Git at query time, so it is never stale; the verdict
    /// describes the indexed rows the churn figures come from.
    pub freshness: Freshness,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IgnoreStatus {
    pub relative_path: String,
    pub ignored: bool,
    pub reason: Option<String>,
    /// The recorded entry that excluded this path, which may be a parent
    /// directory rather than the path itself.
    pub matched_entry: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    pub query: String,
    pub hits: Vec<SearchHit>,
    pub freshness: Vec<Freshness>,
    /// True when any hit rests on a file that has changed since it was indexed.
    pub contains_stale: bool,
}

/// The index service: `index.*`, `search.*`, `symbols.*`, `graph.*`, `history.*`.
pub struct IndexApi {
    storage: Arc<Storage>,
    indexer: Indexer,
    search: SearchService,
    projects: ProjectRepository,
}

impl IndexApi {
    #[must_use]
    pub fn new(storage: Arc<Storage>) -> Self {
        Self {
            indexer: Indexer::new(storage.clone()),
            search: SearchService::new(storage.clone()),
            projects: ProjectRepository::new(storage.clone()),
            storage,
        }
    }

    /// `index.status()`
    pub fn status(&self, repository_id: &str) -> Result<IndexStatus, IndexError> {
        let repository = self.projects.find_repository(repository_id)?;
        let head_oid = history::head_oid(&repository.root_path);
        let branch = history::current_branch(&repository.root_path);

        let counts = self.storage.with_connection(|connection| {
            let count = |sql: &str| -> rusqlite::Result<usize> {
                connection
                    .query_row(sql, [repository_id], |row| row.get::<_, i64>(0))
                    .map(|value| usize::try_from(value).unwrap_or(0))
            };
            Ok(IndexCounts {
                files: count(
                    "SELECT COUNT(*) FROM files WHERE repository_id = ?1 AND deleted_at IS NULL",
                )?,
                symbols: count(
                    "SELECT COUNT(*) FROM symbols s JOIN files f ON f.id = s.file_id
                     WHERE f.repository_id = ?1 AND f.deleted_at IS NULL",
                )?,
                edges: count("SELECT COUNT(*) FROM code_edges WHERE repository_id = ?1")?,
                chunks: count("SELECT COUNT(*) FROM chunks WHERE repository_id = ?1")?,
                commits: count("SELECT COUNT(*) FROM commits WHERE repository_id = ?1")?,
                ignored_paths: count(
                    "SELECT COUNT(*) FROM ignored_paths WHERE repository_id = ?1",
                )?,
            })
        })?;

        let indexed_head_oid: Option<String> = self.storage.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT head_oid FROM index_state WHERE repository_id = ?1",
                    [repository_id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()
                .map(Option::flatten)
        })?;

        let last_run = self.storage.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT id, status, mode, files_seen, indexed_files, skipped_files,
                            deleted_files, renamed_files, chunk_count, started_at, finished_at, error
                     FROM index_runs
                     WHERE repository_id = ?1 AND status != 'running'
                     ORDER BY started_at DESC LIMIT 1",
                    [repository_id],
                    |row| {
                        Ok(IndexRunStatus {
                            run_id: row.get(0)?,
                            status: row.get(1)?,
                            mode: row.get(2)?,
                            files_seen: number(row.get(3)?),
                            indexed_files: number(row.get(4)?),
                            skipped_files: number(row.get(5)?),
                            deleted_files: number(row.get(6)?),
                            renamed_files: number(row.get(7)?),
                            chunk_count: number(row.get(8)?),
                            started_at: row.get(9)?,
                            finished_at: row.get(10)?,
                            error: row.get(11)?,
                        })
                    },
                )
                .optional()
        })?;

        let running_runs = self.storage.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT COUNT(*) FROM index_runs WHERE repository_id = ?1 AND status = 'running'",
                    [repository_id],
                    |row| row.get::<_, i64>(0),
                )
                .map(|value| usize::try_from(value).unwrap_or(0))
        })?;

        Ok(IndexStatus {
            repository_id: repository_id.to_owned(),
            root_path: repository.root_path,
            head_matches_index: match (&head_oid, &indexed_head_oid) {
                (Some(head), Some(indexed)) => head == indexed,
                _ => false,
            },
            branch,
            head_oid,
            indexed_head_oid,
            embedding_model: embedding::MODEL.to_owned(),
            embedding_dimensions: embedding::DIMENSIONS,
            counts,
            last_run,
            running_runs,
        })
    }

    /// `index.reindex(paths?)`
    pub fn reindex(
        &self,
        repository_id: &str,
        paths: Option<Vec<String>>,
        cancellation: &CancellationToken,
    ) -> Result<IndexSummary, IndexError> {
        self.reindex_observing(repository_id, paths, cancellation, |_| {})
    }

    /// `index.reindex(paths?)`, streaming progress events.
    pub fn reindex_observing(
        &self,
        repository_id: &str,
        paths: Option<Vec<String>>,
        cancellation: &CancellationToken,
        observer: impl FnMut(&IndexEvent),
    ) -> Result<IndexSummary, IndexError> {
        let options = paths.map_or_else(IndexOptions::default, IndexOptions::for_paths);
        self.indexer
            .run(repository_id, &options, cancellation, observer)
    }

    /// `search.lexical(query, filters)`
    pub fn search_lexical(
        &self,
        repository_id: &str,
        query: &str,
        filters: &SearchFilters,
        limit: usize,
    ) -> Result<SearchResults, IndexError> {
        let hits = self.search.lexical(repository_id, query, filters, limit)?;
        self.label(repository_id, query, hits)
    }

    /// `search.semantic(query, k, filters)`
    pub fn search_semantic(
        &self,
        repository_id: &str,
        query: &str,
        k: usize,
        filters: &SearchFilters,
    ) -> Result<SearchResults, IndexError> {
        let hits = self.search.semantic(repository_id, query, k, filters)?;
        self.label(repository_id, query, hits)
    }

    /// `search.hybrid(query, k, filters)` — reciprocal-rank fusion of both.
    pub fn search_hybrid(
        &self,
        repository_id: &str,
        query: &str,
        k: usize,
        filters: &SearchFilters,
    ) -> Result<SearchResults, IndexError> {
        let hits = self.search.hybrid(repository_id, query, k, filters)?;
        self.label(repository_id, query, hits)
    }

    /// `symbols.at(path, line)` — innermost definition first.
    pub fn symbols_at(
        &self,
        repository_id: &str,
        relative_path: &str,
        line: usize,
    ) -> Result<Vec<SymbolLocation>, IndexError> {
        let relative_path = relative_path.replace('\\', "/");
        let freshness = self.freshness(repository_id, &relative_path)?;
        let rows: Vec<SymbolLocation> = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT s.id, s.stable_key, s.name, s.kind, s.qualified_name, s.start_line,
                        s.end_line, s.start_byte, s.end_byte, s.signature, s.documentation,
                        s.exported, s.parent_symbol_id, s.blob_sha
                 FROM symbols s JOIN files f ON f.id = s.file_id
                 WHERE f.repository_id = ?1 AND f.relative_path = ?2 AND f.deleted_at IS NULL
                   AND s.start_line <= ?3 AND s.end_line >= ?3
                 ORDER BY (s.end_byte - s.start_byte) ASC",
            )?;
            statement
                .query_map(params![repository_id, relative_path, line as i64], |row| {
                    Ok(SymbolLocation {
                        symbol_id: row.get(0)?,
                        stable_key: row.get(1)?,
                        name: row.get(2)?,
                        kind: row.get(3)?,
                        qualified_name: row.get(4)?,
                        relative_path: String::new(),
                        start_line: number(row.get(5)?),
                        end_line: number(row.get(6)?),
                        start_byte: number(row.get(7)?),
                        end_byte: number(row.get(8)?),
                        signature: row.get(9)?,
                        documentation: row.get(10)?,
                        exported: row.get::<_, i64>(11)? == 1,
                        parent_symbol_id: row.get(12)?,
                        blob_sha: row.get(13)?,
                        freshness: Freshness {
                            state: FreshnessState::Unknown,
                            relative_path: String::new(),
                            indexed_blob_sha: None,
                            current_blob_sha: None,
                        },
                    })
                })?
                .collect()
        })?;

        Ok(rows
            .into_iter()
            .map(|mut symbol| {
                symbol.relative_path = relative_path.clone();
                symbol.freshness = freshness.clone();
                symbol
            })
            .collect())
    }

    /// `graph.neighbors(symbolId, {kinds, depth})`
    pub fn graph_neighbors(
        &self,
        repository_id: &str,
        symbol_id: &str,
        options: &NeighborOptions,
    ) -> Result<Neighborhood, IndexError> {
        let depth = options.depth.clamp(1, MAX_GRAPH_DEPTH);
        let limit = options.limit.clamp(1, MAX_GRAPH_RESULTS);

        let origin: Option<(String, String)> = self.storage.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT f.relative_path, IFNULL(s.blob_sha, '')
                     FROM symbols s JOIN files f ON f.id = s.file_id
                     WHERE s.id = ?1 AND f.repository_id = ?2",
                    params![symbol_id, repository_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
        })?;
        let freshness = match origin {
            Some((path, _)) => self.freshness(repository_id, &path)?,
            None => Freshness {
                state: FreshnessState::Unknown,
                relative_path: String::new(),
                indexed_blob_sha: None,
                current_blob_sha: None,
            },
        };

        let mut nodes: HashMap<String, GraphNode> = HashMap::new();
        let mut edges: Vec<GraphEdge> = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<(String, usize)> = VecDeque::new();
        let mut truncated = false;

        visited.insert(symbol_id.to_owned());
        queue.push_back((symbol_id.to_owned(), 0));

        while let Some((current, distance)) = queue.pop_front() {
            if distance >= depth {
                continue;
            }
            for edge in self.edges_for(repository_id, &current, options)? {
                if edges.len() >= limit {
                    truncated = true;
                    break;
                }
                let next = match edge.direction {
                    Direction::Outgoing => edge.target_id.clone(),
                    _ => Some(edge.source_id.clone()),
                };
                edges.push(edge);
                if let Some(next) = next.filter(|next| next != &current)
                    && visited.insert(next.clone())
                {
                    queue.push_back((next, distance + 1));
                }
            }
            if edges.len() >= limit {
                truncated = true;
                break;
            }
        }

        let ids: Vec<String> = visited.iter().cloned().collect();
        for (id, node) in self.nodes_for(repository_id, &ids)? {
            nodes.insert(id, node);
        }
        // Annotate each node with its hop count from the origin.
        let mut distances: HashMap<String, usize> = HashMap::new();
        distances.insert(symbol_id.to_owned(), 0);
        let mut frontier: VecDeque<String> = VecDeque::from([symbol_id.to_owned()]);
        while let Some(current) = frontier.pop_front() {
            let distance = distances.get(&current).copied().unwrap_or(0);
            for edge in &edges {
                let neighbor = if edge.source_id == current {
                    edge.target_id.clone()
                } else if edge.target_id.as_deref() == Some(current.as_str()) {
                    Some(edge.source_id.clone())
                } else {
                    None
                };
                if let Some(neighbor) = neighbor
                    && !distances.contains_key(&neighbor)
                {
                    distances.insert(neighbor.clone(), distance + 1);
                    frontier.push_back(neighbor);
                }
            }
        }

        let mut nodes: Vec<GraphNode> = nodes
            .into_iter()
            .map(|(id, mut node)| {
                node.distance = distances.get(&id).copied().unwrap_or(usize::MAX);
                node
            })
            .filter(|node| node.distance != usize::MAX)
            .collect();
        nodes.sort_by(|left, right| {
            left.distance
                .cmp(&right.distance)
                .then_with(|| left.relative_path.cmp(&right.relative_path))
                .then_with(|| left.name.cmp(&right.name))
        });

        Ok(Neighborhood {
            symbol_id: symbol_id.to_owned(),
            depth,
            nodes,
            edges,
            truncated,
            freshness,
        })
    }

    /// What a file depends on, and what depends on it.
    pub fn file_dependencies(
        &self,
        repository_id: &str,
        relative_path: &str,
    ) -> Result<FileDependencies, IndexError> {
        let relative_path = relative_path.replace('\\', "/");
        let imports: Vec<String> = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT DISTINCT e.target_key
                 FROM code_edges e JOIN files f ON f.id = e.source_id
                 WHERE e.repository_id = ?1 AND e.source_kind = 'file'
                   AND e.edge_kind IN ('IMPORTS', 'DEPENDS_ON')
                   AND f.relative_path = ?2 AND f.deleted_at IS NULL
                 ORDER BY e.target_key",
            )?;
            statement
                .query_map(params![repository_id, relative_path], |row| row.get(0))?
                .collect()
        })?;

        // An importer is any file whose import specifier resolves to this path
        // by suffix, since specifiers are written without an extension.
        let stem = relative_path
            .rsplit('/')
            .next()
            .and_then(|name| name.split('.').next())
            .unwrap_or(&relative_path)
            .to_owned();
        let imported_by: Vec<String> = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT DISTINCT f.relative_path
                 FROM code_edges e JOIN files f ON f.id = e.source_id
                 WHERE e.repository_id = ?1 AND e.source_kind = 'file'
                   AND e.edge_kind IN ('IMPORTS', 'DEPENDS_ON')
                   AND f.deleted_at IS NULL
                   AND (e.target_key = ?2 OR e.target_key LIKE ?3)
                 ORDER BY f.relative_path",
            )?;
            statement
                .query_map(
                    params![repository_id, relative_path, format!("%/{stem}")],
                    |row| row.get(0),
                )?
                .collect()
        })?;

        Ok(FileDependencies {
            freshness: self.freshness(repository_id, &relative_path)?,
            relative_path,
            imports,
            imported_by,
        })
    }

    /// `history.blame(path, range)`
    pub fn history_blame(
        &self,
        repository_id: &str,
        relative_path: &str,
        start_line: usize,
        end_line: usize,
    ) -> Result<BlameResult, IndexError> {
        let repository = self.projects.find_repository(repository_id)?;
        let relative_path = relative_path.replace('\\', "/");
        let ranges = history::blame(&repository.root_path, &relative_path, start_line, end_line)?;
        Ok(BlameResult {
            churn: history::churn(&self.storage, repository_id, &relative_path)?,
            freshness: self.freshness(repository_id, &relative_path)?,
            relative_path,
            start_line,
            end_line,
            ranges,
        })
    }

    /// Whether a path is excluded from the index, and by which rule.
    pub fn ignore_status(
        &self,
        repository_id: &str,
        relative_path: &str,
    ) -> Result<IgnoreStatus, IndexError> {
        let relative_path = relative_path.replace('\\', "/");
        let matched: Option<(String, String)> = self.storage.with_connection(|connection| {
            connection
                .query_row(
                    // A path is excluded either directly or by a recorded parent
                    // directory such as `node_modules/`.
                    "SELECT relative_path, reason FROM ignored_paths
                     WHERE repository_id = ?1
                       AND (relative_path = ?2
                            OR (entry_kind = 'directory' AND ?2 LIKE relative_path || '/%'))
                     ORDER BY LENGTH(relative_path) DESC LIMIT 1",
                    params![repository_id, relative_path],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
        })?;

        Ok(IgnoreStatus {
            relative_path,
            ignored: matched.is_some(),
            reason: matched.as_ref().map(|matched| matched.1.clone()),
            matched_entry: matched.map(|matched| matched.0),
        })
    }

    /// The freshness verdict for one indexed path.
    pub fn freshness(
        &self,
        repository_id: &str,
        relative_path: &str,
    ) -> Result<Freshness, IndexError> {
        let repository = self.projects.find_repository(repository_id)?;
        let relative_path = relative_path.replace('\\', "/");
        let indexed: Option<String> = self.storage.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT blob_sha FROM files
                     WHERE repository_id = ?1 AND relative_path = ?2 AND deleted_at IS NULL",
                    params![repository_id, relative_path],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()
                .map(Option::flatten)
        })?;
        Ok(freshness_for(
            &repository.root_path,
            &relative_path,
            indexed,
        ))
    }

    /// Every indexed path whose content no longer matches the index.
    pub fn stale_paths(&self, repository_id: &str) -> Result<Vec<Freshness>, IndexError> {
        let repository = self.projects.find_repository(repository_id)?;
        let indexed: Vec<(String, Option<String>)> =
            self.storage.with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT relative_path, blob_sha FROM files
                     WHERE repository_id = ?1 AND deleted_at IS NULL
                     ORDER BY relative_path",
                )?;
                statement
                    .query_map([repository_id], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect()
            })?;

        Ok(indexed
            .into_iter()
            .map(|(path, blob_sha)| freshness_for(&repository.root_path, &path, blob_sha))
            .filter(|freshness| !freshness.is_fresh())
            .collect())
    }

    fn label(
        &self,
        repository_id: &str,
        query: &str,
        hits: Vec<SearchHit>,
    ) -> Result<SearchResults, IndexError> {
        let mut freshness = Vec::new();
        let mut seen = HashSet::new();
        for hit in &hits {
            if seen.insert(hit.relative_path.clone()) {
                freshness.push(self.freshness(repository_id, &hit.relative_path)?);
            }
        }
        Ok(SearchResults {
            query: query.to_owned(),
            contains_stale: freshness.iter().any(|entry| !entry.is_fresh()),
            hits,
            freshness,
        })
    }

    fn edges_for(
        &self,
        repository_id: &str,
        symbol_id: &str,
        options: &NeighborOptions,
    ) -> Result<Vec<GraphEdge>, IndexError> {
        let mut edges = Vec::new();
        if matches!(options.direction, Direction::Outgoing | Direction::Both) {
            edges.extend(self.directed_edges(
                repository_id,
                symbol_id,
                &options.kinds,
                Direction::Outgoing,
            )?);
        }
        if matches!(options.direction, Direction::Incoming | Direction::Both) {
            edges.extend(self.directed_edges(
                repository_id,
                symbol_id,
                &options.kinds,
                Direction::Incoming,
            )?);
        }
        Ok(edges)
    }

    fn directed_edges(
        &self,
        repository_id: &str,
        symbol_id: &str,
        kinds: &[String],
        direction: Direction,
    ) -> Result<Vec<GraphEdge>, IndexError> {
        let anchor = if direction == Direction::Outgoing {
            "e.source_id = ?2"
        } else {
            "e.target_id = ?2"
        };
        let kind_filter = if kinds.is_empty() {
            String::new()
        } else {
            format!(
                " AND e.edge_kind IN ({})",
                vec!["?"; kinds.len()].join(", ")
            )
        };
        let sql = format!(
            "SELECT e.edge_kind, e.source_id, e.source_kind, e.target_id, e.target_key,
                    e.confidence, e.evidence_json
             FROM code_edges e
             WHERE e.repository_id = ?1 AND {anchor}{kind_filter}
             ORDER BY e.edge_kind, e.target_key
             LIMIT {MAX_GRAPH_RESULTS}"
        );

        Ok(self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(&sql)?;
            let mut parameters: Vec<&dyn ToSql> = vec![&repository_id, &symbol_id];
            for kind in kinds {
                parameters.push(kind);
            }
            statement
                .query_map(params_from_iter(parameters), |row| {
                    Ok(GraphEdge {
                        edge_kind: row.get(0)?,
                        source_id: row.get(1)?,
                        source_kind: row.get(2)?,
                        target_id: row.get(3)?,
                        target_key: row.get(4)?,
                        confidence: row.get(5)?,
                        evidence_json: row.get(6)?,
                        direction,
                    })
                })?
                .collect()
        })?)
    }

    fn nodes_for(
        &self,
        repository_id: &str,
        ids: &[String],
    ) -> Result<Vec<(String, GraphNode)>, IndexError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = vec!["?"; ids.len()].join(", ");
        let sql = format!(
            "SELECT s.id, s.name, s.kind, f.relative_path, s.start_line, s.end_line
             FROM symbols s JOIN files f ON f.id = s.file_id
             WHERE f.repository_id = ? AND f.deleted_at IS NULL AND s.id IN ({placeholders})"
        );
        Ok(self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(&sql)?;
            let mut parameters: Vec<&dyn ToSql> = vec![&repository_id];
            for id in ids {
                parameters.push(id);
            }
            statement
                .query_map(params_from_iter(parameters), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        GraphNode {
                            symbol_id: row.get(0)?,
                            name: row.get(1)?,
                            kind: row.get(2)?,
                            relative_path: row.get(3)?,
                            start_line: number(row.get(4)?),
                            end_line: number(row.get(5)?),
                            distance: 0,
                        },
                    ))
                })?
                .collect()
        })?)
    }
}

/// Compares an indexed blob SHA with the file on disk.
#[must_use]
pub fn freshness_for(root: &Path, relative_path: &str, indexed: Option<String>) -> Freshness {
    let absolute = root.join(relative_path);
    if !absolute.is_file() {
        return Freshness {
            state: FreshnessState::Missing,
            relative_path: relative_path.to_owned(),
            indexed_blob_sha: indexed,
            current_blob_sha: None,
        };
    }
    let current = walk::blob_sha(&absolute).ok();
    let state = match (indexed.as_deref(), current.as_deref()) {
        (Some(indexed), Some(current)) if indexed == current => FreshnessState::Fresh,
        (Some(_), Some(_)) => FreshnessState::Stale,
        _ => FreshnessState::Unknown,
    };
    Freshness {
        state,
        relative_path: relative_path.to_owned(),
        indexed_blob_sha: indexed,
        current_blob_sha: current,
    }
}

fn number(value: i64) -> usize {
    usize::try_from(value).unwrap_or(0)
}
