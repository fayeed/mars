use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use mars_storage::Storage;
use rusqlite::params;
use thiserror::Error;

use crate::tools::{BrainEdge, BrainNeighborhood, BrainSymbol, Dependency, EdgeEvidence};

const MAX_RESULTS: usize = 100;
const MAX_DEPTH: usize = 5;

#[derive(Debug, Error)]
pub enum BrainError {
    #[error(transparent)]
    Storage(#[from] mars_storage::StorageError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("query bounds are invalid: depth {depth}, limit {limit}")]
    InvalidBounds { depth: usize, limit: usize },
    #[error("symbol `{0}` does not exist in this repository")]
    SymbolNotFound(String),
}

pub struct BrainService {
    storage: Arc<Storage>,
}

impl BrainService {
    #[must_use]
    pub fn new(storage: Arc<Storage>) -> Self {
        Self { storage }
    }

    pub fn find_symbol(
        &self,
        repository_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<BrainSymbol>, BrainError> {
        validate_limit(limit)?;
        let pattern = format!("%{}%", query.trim());
        Ok(self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT s.id, s.name, s.kind, s.qualified_name, f.relative_path,
                        s.start_line, s.end_line, s.exported
                 FROM symbols s JOIN files f ON f.id = s.file_id
                 WHERE f.repository_id = ?1 AND f.deleted_at IS NULL
                   AND (s.name LIKE ?2 OR s.qualified_name LIKE ?2)
                 ORDER BY CASE WHEN s.name = ?3 THEN 0 ELSE 1 END, s.name, f.relative_path
                 LIMIT ?4",
            )?;
            statement
                .query_map(
                    params![repository_id, pattern, query.trim(), limit as i64],
                    symbol_from_row,
                )?
                .collect()
        })?)
    }

    pub fn references(
        &self,
        repository_id: &str,
        symbol_id: &str,
        limit: usize,
    ) -> Result<Vec<BrainEdge>, BrainError> {
        self.edges(
            repository_id,
            "e.target_id = ?2 AND e.edge_kind = 'REFERENCES'",
            symbol_id,
            limit,
        )
    }

    pub fn callers(
        &self,
        repository_id: &str,
        symbol_id: &str,
        limit: usize,
    ) -> Result<Vec<BrainEdge>, BrainError> {
        self.edges(
            repository_id,
            "e.target_id = ?2 AND e.edge_kind = 'CALLS'",
            symbol_id,
            limit,
        )
    }

    pub fn callees(
        &self,
        repository_id: &str,
        symbol_id: &str,
        limit: usize,
    ) -> Result<Vec<BrainEdge>, BrainError> {
        self.edges(
            repository_id,
            "e.source_id = ?2 AND e.edge_kind = 'CALLS'",
            symbol_id,
            limit,
        )
    }

    pub fn dependencies(
        &self,
        repository_id: &str,
        limit: usize,
    ) -> Result<Vec<Dependency>, BrainError> {
        validate_limit(limit)?;
        let raw: Vec<(String, String, f64, String)> =
            self.storage.with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT f.relative_path, e.target_key, e.confidence, e.evidence_json
                 FROM code_edges e JOIN files f ON e.source_kind = 'file' AND f.id = e.source_id
                 WHERE e.repository_id = ?1 AND e.edge_kind = 'IMPORTS' AND f.deleted_at IS NULL
                 ORDER BY f.relative_path, e.target_key LIMIT ?2",
                )?;
                statement
                    .query_map(params![repository_id, limit as i64], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                    })?
                    .collect()
            })?;
        raw.into_iter()
            .map(|(relative_path, specifier, confidence, evidence)| {
                Ok(Dependency {
                    relative_path,
                    specifier,
                    confidence,
                    evidence: serde_json::from_str(&evidence)?,
                })
            })
            .collect()
    }

    pub fn related_code(
        &self,
        repository_id: &str,
        symbol_id: &str,
        depth: usize,
        limit: usize,
    ) -> Result<BrainNeighborhood, BrainError> {
        self.neighborhood(repository_id, symbol_id, depth, limit, false)
    }

    pub fn blast_radius(
        &self,
        repository_id: &str,
        symbol_id: &str,
        depth: usize,
        limit: usize,
    ) -> Result<BrainNeighborhood, BrainError> {
        self.neighborhood(repository_id, symbol_id, depth, limit, true)
    }

    fn edges(
        &self,
        repository_id: &str,
        predicate: &str,
        symbol_id: &str,
        limit: usize,
    ) -> Result<Vec<BrainEdge>, BrainError> {
        validate_limit(limit)?;
        let sql = format!(
            "{} WHERE e.repository_id = ?1 AND {predicate} ORDER BY e.confidence DESC LIMIT ?3",
            edge_select()
        );
        let raw: Vec<RawEdge> = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(&sql)?;
            statement
                .query_map(
                    params![repository_id, symbol_id, limit as i64],
                    raw_edge_from_row,
                )?
                .collect()
        })?;
        raw.into_iter().map(BrainEdge::try_from).collect()
    }

    fn neighborhood(
        &self,
        repository_id: &str,
        symbol_id: &str,
        depth: usize,
        limit: usize,
        reverse_only: bool,
    ) -> Result<BrainNeighborhood, BrainError> {
        validate_bounds(depth, limit)?;
        let root = self
            .symbol(repository_id, symbol_id)?
            .ok_or_else(|| BrainError::SymbolNotFound(symbol_id.into()))?;
        let mut visited = HashSet::from([symbol_id.to_owned()]);
        let mut queue = VecDeque::from([(symbol_id.to_owned(), 0_usize)]);
        let mut symbols = Vec::new();
        let mut edges = Vec::new();

        while let Some((current, level)) = queue.pop_front() {
            if level >= depth || symbols.len() >= limit {
                continue;
            }
            let adjacent = self.adjacent_edges(repository_id, &current, reverse_only, limit)?;
            for edge in adjacent {
                let next = if reverse_only {
                    edge.source.clone()
                } else if edge
                    .source
                    .as_ref()
                    .is_some_and(|source| source.id == current)
                {
                    edge.target.clone()
                } else {
                    edge.source.clone()
                };
                edges.push(edge);
                if let Some(next) = next
                    && visited.insert(next.id.clone())
                    && symbols.len() < limit
                {
                    queue.push_back((next.id.clone(), level + 1));
                    symbols.push(next);
                }
                if edges.len() >= limit {
                    break;
                }
            }
            if edges.len() >= limit {
                break;
            }
        }
        Ok(BrainNeighborhood {
            root,
            symbols,
            edges,
        })
    }

    fn adjacent_edges(
        &self,
        repository_id: &str,
        symbol_id: &str,
        reverse_only: bool,
        limit: usize,
    ) -> Result<Vec<BrainEdge>, BrainError> {
        let predicate = if reverse_only {
            "e.target_id = ?2 AND e.edge_kind IN ('CALLS', 'REFERENCES', 'EXTENDS')"
        } else {
            "(e.source_id = ?2 OR e.target_id = ?2) AND e.target_id IS NOT NULL"
        };
        self.edges(repository_id, predicate, symbol_id, limit)
    }

    fn symbol(
        &self,
        repository_id: &str,
        symbol_id: &str,
    ) -> Result<Option<BrainSymbol>, BrainError> {
        Ok(self.storage.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT s.id, s.name, s.kind, s.qualified_name, f.relative_path,
                            s.start_line, s.end_line, s.exported
                     FROM symbols s JOIN files f ON f.id = s.file_id
                     WHERE f.repository_id = ?1 AND f.deleted_at IS NULL AND s.id = ?2",
                    params![repository_id, symbol_id],
                    symbol_from_row,
                )
                .optional()
        })?)
    }
}

use rusqlite::OptionalExtension;

fn validate_limit(limit: usize) -> Result<(), BrainError> {
    if limit == 0 || limit > MAX_RESULTS {
        return Err(BrainError::InvalidBounds { depth: 1, limit });
    }
    Ok(())
}

fn validate_bounds(depth: usize, limit: usize) -> Result<(), BrainError> {
    if depth == 0 || depth > MAX_DEPTH || limit == 0 || limit > MAX_RESULTS {
        return Err(BrainError::InvalidBounds { depth, limit });
    }
    Ok(())
}

fn symbol_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<BrainSymbol> {
    Ok(BrainSymbol {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: row.get(2)?,
        qualified_name: row.get(3)?,
        relative_path: row.get(4)?,
        start_line: row.get::<_, i64>(5)? as usize,
        end_line: row.get::<_, i64>(6)? as usize,
        exported: row.get(7)?,
    })
}

fn edge_select() -> &'static str {
    "SELECT e.id, e.edge_kind, e.target_key, e.confidence, e.evidence_json,
            ss.id, ss.name, ss.kind, ss.qualified_name, sf.relative_path, ss.start_line, ss.end_line, ss.exported,
            ts.id, ts.name, ts.kind, ts.qualified_name, tf.relative_path, ts.start_line, ts.end_line, ts.exported
     FROM code_edges e
     LEFT JOIN symbols ss ON e.source_kind = 'symbol' AND ss.id = e.source_id
     LEFT JOIN files sf ON sf.id = ss.file_id
     LEFT JOIN symbols ts ON ts.id = e.target_id
     LEFT JOIN files tf ON tf.id = ts.file_id"
}

struct RawEdge {
    id: String,
    edge_kind: String,
    target_key: String,
    confidence: f64,
    evidence: String,
    source: Option<BrainSymbol>,
    target: Option<BrainSymbol>,
}

fn raw_edge_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawEdge> {
    Ok(RawEdge {
        id: row.get(0)?,
        edge_kind: row.get(1)?,
        target_key: row.get(2)?,
        confidence: row.get(3)?,
        evidence: row.get(4)?,
        source: optional_symbol(row, 5)?,
        target: optional_symbol(row, 13)?,
    })
}

fn optional_symbol(
    row: &rusqlite::Row<'_>,
    offset: usize,
) -> rusqlite::Result<Option<BrainSymbol>> {
    let Some(id) = row.get::<_, Option<String>>(offset)? else {
        return Ok(None);
    };
    Ok(Some(BrainSymbol {
        id,
        name: row.get(offset + 1)?,
        kind: row.get(offset + 2)?,
        qualified_name: row.get(offset + 3)?,
        relative_path: row.get(offset + 4)?,
        start_line: row.get::<_, i64>(offset + 5)? as usize,
        end_line: row.get::<_, i64>(offset + 6)? as usize,
        exported: row.get(offset + 7)?,
    }))
}

impl TryFrom<RawEdge> for BrainEdge {
    type Error = BrainError;

    fn try_from(raw: RawEdge) -> Result<Self, Self::Error> {
        Ok(Self {
            id: raw.id,
            edge_kind: raw.edge_kind,
            source: raw.source,
            target: raw.target,
            target_key: raw.target_key,
            confidence: raw.confidence,
            evidence: serde_json::from_str::<EdgeEvidence>(&raw.evidence)?,
        })
    }
}
