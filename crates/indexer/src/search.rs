//! Lexical, semantic, and hybrid search over the chunk layer.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use mars_storage::Storage;
use rusqlite::{OptionalExtension, ToSql, params_from_iter};
use serde::Serialize;

use crate::IndexError;
use crate::embedding;

/// Reciprocal-rank fusion constant. 60 is the value the original RRF paper
/// settled on, and it keeps a strong hit from either list near the top without
/// letting one list dominate.
const RRF_CONSTANT: f64 = 60.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Retrieval {
    Lexical,
    Semantic,
    Hybrid,
}

#[derive(Clone, Debug, Default)]
pub struct SearchFilters {
    /// Repository-relative path prefixes; empty means the whole repository.
    pub path_prefixes: Vec<String>,
    pub languages: Vec<String>,
}

impl SearchFilters {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.path_prefixes.is_empty() && self.languages.is_empty()
    }

    fn matches(&self, relative_path: &str, language: &str) -> bool {
        let path_ok = self.path_prefixes.is_empty()
            || self
                .path_prefixes
                .iter()
                .any(|prefix| relative_path.starts_with(prefix.as_str()));
        let language_ok =
            self.languages.is_empty() || self.languages.iter().any(|value| value == language);
        path_ok && language_ok
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub chunk_id: String,
    pub relative_path: String,
    pub symbol_path: String,
    pub header: String,
    pub body: String,
    pub language: String,
    pub start_line: usize,
    pub end_line: usize,
    pub blob_sha: String,
    pub score: f64,
    pub retrieval: Retrieval,
    pub lexical_rank: Option<usize>,
    pub semantic_rank: Option<usize>,
}

#[derive(Clone, Debug)]
struct ChunkRow {
    chunk_id: String,
    relative_path: String,
    symbol_path: String,
    header: String,
    body: String,
    language: String,
    start_line: usize,
    end_line: usize,
    blob_sha: String,
}

#[derive(Clone, Debug)]
struct VectorRow {
    chunk_id: String,
    relative_path: String,
    language: String,
    vector: Vec<f32>,
}

struct VectorCache {
    run_id: Option<String>,
    rows: Arc<Vec<VectorRow>>,
}

/// Reads the chunk layer. Vectors are cached in memory per repository and
/// refreshed whenever a new index run completes.
pub struct SearchService {
    storage: Arc<Storage>,
    vectors: Mutex<HashMap<String, VectorCache>>,
}

impl SearchService {
    #[must_use]
    pub fn new(storage: Arc<Storage>) -> Self {
        Self {
            storage,
            vectors: Mutex::new(HashMap::new()),
        }
    }

    /// Full-text search ranked by BM25.
    pub fn lexical(
        &self,
        repository_id: &str,
        query: &str,
        filters: &SearchFilters,
        limit: usize,
    ) -> Result<Vec<SearchHit>, IndexError> {
        let Some(expression) = match_expression(query) else {
            return Ok(Vec::new());
        };
        let mut hits = self.lexical_rows(repository_id, &expression, filters, limit)?;
        if hits.is_empty() {
            // Fall back from "every term" to "any term" before reporting nothing.
            if let Some(any) = any_term_expression(query) {
                hits = self.lexical_rows(repository_id, &any, filters, limit)?;
            }
        }
        Ok(hits)
    }

    fn lexical_rows(
        &self,
        repository_id: &str,
        expression: &str,
        filters: &SearchFilters,
        limit: usize,
    ) -> Result<Vec<SearchHit>, IndexError> {
        // Over-fetch so post-filtering cannot empty a page of results.
        let fetch = limit.saturating_mul(4).max(limit).min(1000);
        let rows: Vec<(ChunkRow, f64)> = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT c.id, f.relative_path, c.symbol_path, c.header, c.body, f.language,
                        c.start_line, c.end_line, c.blob_sha, bm25(chunk_search) AS rank
                 FROM chunk_search
                 JOIN chunks c ON c.id = chunk_search.chunk_id
                 JOIN files f ON f.id = c.file_id
                 WHERE chunk_search MATCH ?1
                   AND chunk_search.repository_id = ?2
                   AND f.deleted_at IS NULL
                 ORDER BY rank
                 LIMIT ?3",
            )?;
            statement
                .query_map(
                    rusqlite::params![expression, repository_id, fetch as i64],
                    |row| Ok((chunk_row(row)?, row.get::<_, f64>(9)?)),
                )?
                .collect()
        })?;

        Ok(rows
            .into_iter()
            .filter(|(row, _)| filters.matches(&row.relative_path, &row.language))
            .take(limit)
            .enumerate()
            .map(|(rank, (row, bm25))| SearchHit {
                // BM25 is reported as a negative number; invert it so a larger
                // score is always a better match.
                score: -bm25,
                retrieval: Retrieval::Lexical,
                lexical_rank: Some(rank + 1),
                semantic_rank: None,
                chunk_id: row.chunk_id,
                relative_path: row.relative_path,
                symbol_path: row.symbol_path,
                header: row.header,
                body: row.body,
                language: row.language,
                start_line: row.start_line,
                end_line: row.end_line,
                blob_sha: row.blob_sha,
            })
            .collect())
    }

    /// Nearest chunks by cosine similarity in the local embedding space.
    pub fn semantic(
        &self,
        repository_id: &str,
        query: &str,
        k: usize,
        filters: &SearchFilters,
    ) -> Result<Vec<SearchHit>, IndexError> {
        if query.trim().is_empty() || k == 0 {
            return Ok(Vec::new());
        }
        let vectors = self.vectors(repository_id)?;
        let query_vector = embedding::embed(query);

        let mut scored: Vec<(f32, &VectorRow)> = vectors
            .iter()
            .filter(|row| filters.matches(&row.relative_path, &row.language))
            .map(|row| (embedding::similarity(&query_vector, &row.vector), row))
            .filter(|(score, _)| *score > 0.0)
            .collect();
        scored.sort_by(|left, right| {
            right
                .0
                .partial_cmp(&left.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.1.chunk_id.cmp(&right.1.chunk_id))
        });
        scored.truncate(k);

        let ids: Vec<String> = scored.iter().map(|(_, row)| row.chunk_id.clone()).collect();
        let details = self.chunk_details(&ids)?;

        Ok(scored
            .into_iter()
            .enumerate()
            .filter_map(|(rank, (score, row))| {
                let detail = details.get(&row.chunk_id)?;
                Some(SearchHit {
                    score: f64::from(score),
                    retrieval: Retrieval::Semantic,
                    lexical_rank: None,
                    semantic_rank: Some(rank + 1),
                    chunk_id: detail.chunk_id.clone(),
                    relative_path: detail.relative_path.clone(),
                    symbol_path: detail.symbol_path.clone(),
                    header: detail.header.clone(),
                    body: detail.body.clone(),
                    language: detail.language.clone(),
                    start_line: detail.start_line,
                    end_line: detail.end_line,
                    blob_sha: detail.blob_sha.clone(),
                })
            })
            .collect())
    }

    /// Reciprocal-rank fusion of the lexical and semantic lists.
    pub fn hybrid(
        &self,
        repository_id: &str,
        query: &str,
        k: usize,
        filters: &SearchFilters,
    ) -> Result<Vec<SearchHit>, IndexError> {
        // Fuse deeper lists than requested so a hit ranked well by only one
        // retriever can still surface.
        let depth = k.saturating_mul(3).max(k);
        let lexical = self.lexical(repository_id, query, filters, depth)?;
        let semantic = self.semantic(repository_id, query, depth, filters)?;

        let mut fused: HashMap<String, SearchHit> = HashMap::new();
        let mut scores: HashMap<String, f64> = HashMap::new();

        for hit in lexical {
            let rank = hit.lexical_rank.unwrap_or(depth + 1);
            *scores.entry(hit.chunk_id.clone()).or_default() += 1.0 / (RRF_CONSTANT + rank as f64);
            fused.insert(hit.chunk_id.clone(), hit);
        }
        for hit in semantic {
            let rank = hit.semantic_rank.unwrap_or(depth + 1);
            *scores.entry(hit.chunk_id.clone()).or_default() += 1.0 / (RRF_CONSTANT + rank as f64);
            fused
                .entry(hit.chunk_id.clone())
                .and_modify(|existing| existing.semantic_rank = hit.semantic_rank)
                .or_insert(hit);
        }

        let mut hits: Vec<SearchHit> = fused
            .into_values()
            .map(|mut hit| {
                hit.score = scores.get(&hit.chunk_id).copied().unwrap_or_default();
                hit.retrieval = Retrieval::Hybrid;
                hit
            })
            .collect();
        hits.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.chunk_id.cmp(&right.chunk_id))
        });
        hits.truncate(k);
        Ok(hits)
    }

    fn chunk_details(&self, ids: &[String]) -> Result<HashMap<String, ChunkRow>, IndexError> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let placeholders = vec!["?"; ids.len()].join(", ");
        let sql = format!(
            "SELECT c.id, f.relative_path, c.symbol_path, c.header, c.body, f.language,
                    c.start_line, c.end_line, c.blob_sha
             FROM chunks c JOIN files f ON f.id = c.file_id
             WHERE c.id IN ({placeholders})"
        );
        let rows: Vec<ChunkRow> = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(&sql)?;
            let parameters = ids.iter().map(|id| id as &dyn ToSql);
            statement
                .query_map(params_from_iter(parameters), chunk_row)?
                .collect()
        })?;
        Ok(rows
            .into_iter()
            .map(|row| (row.chunk_id.clone(), row))
            .collect())
    }

    fn vectors(&self, repository_id: &str) -> Result<Arc<Vec<VectorRow>>, IndexError> {
        let run_id: Option<String> = self.storage.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT last_run_id FROM index_state WHERE repository_id = ?1",
                    [repository_id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()
                .map(Option::flatten)
        })?;

        let mut cache = self
            .vectors
            .lock()
            .map_err(|_| IndexError::Walk("vector cache lock was poisoned".into()))?;
        if let Some(entry) = cache.get(repository_id)
            && entry.run_id == run_id
            && run_id.is_some()
        {
            return Ok(entry.rows.clone());
        }

        let rows: Vec<VectorRow> = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT e.chunk_id, f.relative_path, f.language, e.vector
                 FROM chunk_embeddings e
                 JOIN chunks c ON c.id = e.chunk_id
                 JOIN files f ON f.id = c.file_id
                 WHERE e.repository_id = ?1 AND e.model = ?2 AND f.deleted_at IS NULL",
            )?;
            statement
                .query_map(rusqlite::params![repository_id, embedding::MODEL], |row| {
                    Ok(VectorRow {
                        chunk_id: row.get(0)?,
                        relative_path: row.get(1)?,
                        language: row.get(2)?,
                        vector: embedding::decode(&row.get::<_, Vec<u8>>(3)?).unwrap_or_default(),
                    })
                })?
                .collect()
        })?;

        let rows = Arc::new(rows);
        cache.insert(
            repository_id.to_owned(),
            VectorCache {
                run_id,
                rows: rows.clone(),
            },
        );
        Ok(rows)
    }
}

fn chunk_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChunkRow> {
    Ok(ChunkRow {
        chunk_id: row.get(0)?,
        relative_path: row.get(1)?,
        symbol_path: row.get(2)?,
        header: row.get(3)?,
        body: row.get(4)?,
        language: row.get(5)?,
        start_line: usize::try_from(row.get::<_, i64>(6)?).unwrap_or(0),
        end_line: usize::try_from(row.get::<_, i64>(7)?).unwrap_or(0),
        blob_sha: row.get(8)?,
    })
}

/// Builds an FTS5 expression that requires every term. User input is quoted, so
/// punctuation in a query can never be read as FTS5 syntax.
fn match_expression(query: &str) -> Option<String> {
    let terms = terms(query);
    if terms.is_empty() {
        return None;
    }
    Some(
        terms
            .iter()
            .map(|term| format!("\"{term}\""))
            .collect::<Vec<_>>()
            .join(" AND "),
    )
}

fn any_term_expression(query: &str) -> Option<String> {
    let terms = terms(query);
    if terms.len() < 2 {
        return None;
    }
    Some(
        terms
            .iter()
            .map(|term| format!("\"{term}\""))
            .collect::<Vec<_>>()
            .join(" OR "),
    )
}

fn terms(query: &str) -> Vec<String> {
    query
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .filter(|term| !term.is_empty())
        .map(str::to_lowercase)
        .collect()
}
