//! Atomic publication of one index run.
//!
//! Everything a run produces is computed before the transaction opens, so the
//! write lock is held only for the replacement itself and an interrupted run
//! never exposes a partial file.

use std::collections::HashMap;
use std::sync::Arc;

use mars_storage::Storage;
use rusqlite::{OptionalExtension, Transaction, params};
use uuid::Uuid;

use crate::IndexError;
use crate::chunking::ChunkFact;
use crate::embedding;
use crate::facts::ParsedFile;
use crate::timestamp;
use crate::walk::IgnoredPath;

#[derive(Clone, Debug)]
pub struct PreparedChunk {
    pub fact: ChunkFact,
    pub vector: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct PreparedFile {
    pub parsed: ParsedFile,
    pub chunks: Vec<PreparedChunk>,
}

/// A path whose content moved rather than being deleted and re-added.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Rename {
    pub from: String,
    pub to: String,
}

pub struct PublishInput<'a> {
    pub repository_id: &'a str,
    pub run_id: &'a str,
    pub files: &'a [PreparedFile],
    pub deleted: &'a [String],
    pub renames: &'a [Rename],
    pub ignored: Option<&'a [IgnoredPath]>,
    pub files_seen: usize,
    pub skipped_files: usize,
    pub head_oid: Option<&'a str>,
    pub branch: Option<&'a str>,
    pub history_head_oid: Option<&'a str>,
}

pub fn publish(storage: &Arc<Storage>, input: &PublishInput<'_>) -> Result<(), IndexError> {
    let indexed_at = timestamp()?;
    let chunk_count: usize = input.files.iter().map(|file| file.chunks.len()).sum();

    storage.with_transaction(|transaction| {
        // Renames first: the file keeps its identity, so symbol ids downstream of
        // it stay stable across the move.
        for rename in input.renames {
            transaction.execute(
                "UPDATE files SET relative_path = ?3, renamed_from = ?2, deleted_at = NULL
                 WHERE repository_id = ?1 AND relative_path = ?2",
                params![input.repository_id, rename.from, rename.to],
            )?;
        }
        // Only the names this run disturbed can change how an edge resolves, so
        // reconciliation is scoped to them rather than to the whole repository.
        let mut touched = Touched::default();
        for relative_path in input.deleted {
            retire_file(
                transaction,
                input.repository_id,
                relative_path,
                &indexed_at,
                &mut touched,
            )?;
        }
        for file in input.files {
            replace_file(
                transaction,
                input.repository_id,
                input.run_id,
                file,
                &indexed_at,
                &mut touched,
            )?;
        }
        reconcile_targets(transaction, input.repository_id, &touched)?;
        if let Some(ignored) = input.ignored {
            replace_ignored(transaction, input.repository_id, ignored, &indexed_at)?;
        }
        record_state(transaction, input, &indexed_at)?;

        transaction.execute(
            "UPDATE index_runs SET
               status = 'completed',
               files_seen = ?2,
               indexed_files = ?3,
               skipped_files = ?4,
               deleted_files = ?5,
               renamed_files = ?6,
               chunk_count = ?7,
               finished_at = ?8
             WHERE id = ?1",
            params![
                input.run_id,
                input.files_seen as i64,
                input.files.len() as i64,
                input.skipped_files as i64,
                input.deleted.len() as i64,
                input.renames.len() as i64,
                chunk_count as i64,
                indexed_at,
            ],
        )?;
        Ok(())
    })?;
    Ok(())
}

/// The names and symbol ids a run disturbed, which bound the work that edge
/// reconciliation has to do.
#[derive(Debug, Default)]
struct Touched {
    names: std::collections::HashSet<String>,
    removed_symbol_ids: Vec<String>,
}

fn replace_file(
    transaction: &Transaction<'_>,
    repository_id: &str,
    run_id: &str,
    prepared: &PreparedFile,
    indexed_at: &str,
    touched: &mut Touched,
) -> rusqlite::Result<()> {
    let parsed = &prepared.parsed;
    let existing_id: Option<String> = transaction
        .query_row(
            "SELECT id FROM files WHERE repository_id = ?1 AND relative_path = ?2",
            params![repository_id, parsed.source.relative_path],
            |row| row.get(0),
        )
        .optional()?;
    let file_id = existing_id.unwrap_or_else(|| Uuid::now_v7().to_string());
    let existing_symbols = symbol_ids(transaction, &file_id)?;
    for (name, symbol_id) in previous_names(transaction, &file_id)? {
        touched.names.insert(name);
        touched.removed_symbol_ids.push(symbol_id);
    }
    for symbol in &parsed.symbols {
        touched.names.insert(symbol.name.clone());
    }
    for edge in &parsed.edges {
        if edge.target_kind == "symbol" {
            touched.names.insert(edge.target_key.clone());
        }
    }

    for symbol_id in existing_symbols.values() {
        transaction
            .prepare_cached("DELETE FROM code_edges WHERE source_id = ?1")?
            .execute([symbol_id])?;
    }
    transaction
        .prepare_cached("DELETE FROM code_edges WHERE source_kind = 'file' AND source_id = ?1")?
        .execute([&file_id])?;
    clear_chunks(transaction, &file_id)?;
    transaction
        .prepare_cached("DELETE FROM symbols WHERE file_id = ?1")?
        .execute([&file_id])?;
    transaction
        .prepare_cached(
            "INSERT INTO files
         (id, repository_id, relative_path, language, content_hash, byte_size, mtime_ns,
          parse_errors, indexed_at, deleted_at, blob_sha, is_binary)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?11)
         ON CONFLICT(repository_id, relative_path) DO UPDATE SET
           language = excluded.language,
           content_hash = excluded.content_hash,
           byte_size = excluded.byte_size,
           mtime_ns = excluded.mtime_ns,
           parse_errors = excluded.parse_errors,
           indexed_at = excluded.indexed_at,
           blob_sha = excluded.blob_sha,
           is_binary = excluded.is_binary,
           deleted_at = NULL",
        )?
        .execute(params![
            file_id,
            repository_id,
            parsed.source.relative_path,
            parsed.source.language,
            parsed.source.content_hash,
            parsed.source.byte_size as i64,
            parsed.source.mtime_ns,
            parsed.parse_errors as i64,
            indexed_at,
            parsed.source.blob_sha,
            i64::from(parsed.source.is_binary),
        ])?;

    let mut symbol_by_key = HashMap::new();
    for symbol in &parsed.symbols {
        let symbol_id = existing_symbols
            .get(&symbol.stable_key)
            .cloned()
            .unwrap_or_else(|| Uuid::now_v7().to_string());
        // A definition is always visited before the definitions it encloses, so
        // its database id is already known here.
        let parent_symbol_id = symbol
            .parent_stable_key
            .as_ref()
            .and_then(|key| symbol_by_key.get(key).cloned());
        transaction
            .prepare_cached(
                "INSERT INTO symbols
             (id, file_id, stable_key, kind, name, qualified_name, start_byte, end_byte,
              start_line, end_line, signature, exported, content_hash,
              parent_stable_key, parent_symbol_id, documentation, blob_sha)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            )?
            .execute(params![
                symbol_id,
                file_id,
                symbol.stable_key,
                symbol.kind,
                symbol.name,
                symbol.qualified_name,
                symbol.start_byte as i64,
                symbol.end_byte as i64,
                symbol.start_line as i64,
                symbol.end_line as i64,
                symbol.signature,
                i64::from(symbol.exported),
                parsed.source.content_hash,
                symbol.parent_stable_key,
                parent_symbol_id,
                symbol.documentation,
                parsed.source.blob_sha,
            ])?;
        symbol_by_key.insert(symbol.stable_key.clone(), symbol_id);
    }

    for edge in &parsed.edges {
        let (source_kind, source_id) = edge
            .source_stable_key
            .as_ref()
            .and_then(|key| symbol_by_key.get(key))
            .map_or(("file", file_id.as_str()), |id| ("symbol", id.as_str()));
        let target_id = unique_target(transaction, repository_id, &edge.target_key)?;
        let confidence = if target_id.is_some() {
            edge.confidence.max(0.9)
        } else {
            edge.confidence
        };
        transaction
            .prepare_cached(
                "INSERT OR REPLACE INTO code_edges
             (id, repository_id, source_kind, source_id, edge_kind, target_kind, target_id,
              target_key, confidence, evidence_json, index_run_id, blob_sha)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            )?
            .execute(params![
                Uuid::now_v7().to_string(),
                repository_id,
                source_kind,
                source_id,
                edge.edge_kind,
                edge.target_kind,
                target_id,
                edge.target_key,
                confidence,
                edge.evidence_json,
                run_id,
                parsed.source.blob_sha,
            ])?;
    }

    for chunk in &prepared.chunks {
        let symbol_id = chunk
            .fact
            .symbol_stable_key
            .as_ref()
            .and_then(|key| symbol_by_key.get(key).cloned());
        transaction
            .prepare_cached(
                "INSERT INTO chunks
             (id, repository_id, file_id, symbol_id, ordinal, symbol_path, header, body,
              start_byte, end_byte, start_line, end_line, blob_sha, content_hash, indexed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT(id) DO UPDATE SET
               file_id = excluded.file_id,
               symbol_id = excluded.symbol_id,
               ordinal = excluded.ordinal,
               header = excluded.header,
               body = excluded.body,
               start_byte = excluded.start_byte,
               end_byte = excluded.end_byte,
               start_line = excluded.start_line,
               end_line = excluded.end_line,
               blob_sha = excluded.blob_sha,
               indexed_at = excluded.indexed_at",
            )?
            .execute(params![
                chunk.fact.id,
                repository_id,
                file_id,
                symbol_id,
                chunk.fact.ordinal as i64,
                chunk.fact.symbol_path,
                chunk.fact.header,
                chunk.fact.body,
                chunk.fact.start_byte as i64,
                chunk.fact.end_byte as i64,
                chunk.fact.start_line as i64,
                chunk.fact.end_line as i64,
                parsed.source.blob_sha,
                chunk.fact.content_hash,
                indexed_at,
            ])?;
        transaction.prepare_cached("INSERT INTO chunk_embeddings (chunk_id, repository_id, model, dimensions, vector, blob_sha)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(chunk_id) DO UPDATE SET
               model = excluded.model,
               dimensions = excluded.dimensions,
               vector = excluded.vector,
               blob_sha = excluded.blob_sha")?.execute(params![
                chunk.fact.id,
                repository_id,
                embedding::MODEL,
                embedding::DIMENSIONS as i64,
                embedding::encode(&chunk.vector),
                parsed.source.blob_sha,
            ],
        )?;
        transaction
            .prepare_cached(
                "INSERT INTO chunk_search (chunk_id, repository_id, relative_path, header, body)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            )?
            .execute(params![
                chunk.fact.id,
                repository_id,
                parsed.source.relative_path,
                chunk.fact.header,
                chunk.fact.body,
            ])?;
    }
    Ok(())
}

fn retire_file(
    transaction: &Transaction<'_>,
    repository_id: &str,
    relative_path: &str,
    deleted_at: &str,
    touched: &mut Touched,
) -> rusqlite::Result<()> {
    let file_id: Option<String> = transaction
        .query_row(
            "SELECT id FROM files WHERE repository_id = ?1 AND relative_path = ?2",
            params![repository_id, relative_path],
            |row| row.get(0),
        )
        .optional()?;
    let Some(file_id) = file_id else {
        return Ok(());
    };
    for (name, symbol_id) in previous_names(transaction, &file_id)? {
        touched.names.insert(name);
        touched.removed_symbol_ids.push(symbol_id);
    }
    let symbols = symbol_ids(transaction, &file_id)?;
    for symbol_id in symbols.values() {
        transaction
            .prepare_cached("DELETE FROM code_edges WHERE source_id = ?1 OR target_id = ?1")?
            .execute([symbol_id])?;
    }
    transaction
        .prepare_cached("DELETE FROM code_edges WHERE source_kind = 'file' AND source_id = ?1")?
        .execute([&file_id])?;
    clear_chunks(transaction, &file_id)?;
    transaction
        .prepare_cached("DELETE FROM symbols WHERE file_id = ?1")?
        .execute([&file_id])?;
    transaction
        .prepare_cached("UPDATE files SET deleted_at = ?2 WHERE id = ?1")?
        .execute(params![file_id, deleted_at])?;
    Ok(())
}

/// The full-text index is a virtual table, so its rows are removed explicitly
/// rather than by foreign-key cascade.
fn clear_chunks(transaction: &Transaction<'_>, file_id: &str) -> rusqlite::Result<()> {
    transaction
        .prepare_cached(
            "DELETE FROM chunk_search WHERE chunk_id IN (SELECT id FROM chunks WHERE file_id = ?1)",
        )?
        .execute([file_id])?;
    transaction
        .prepare_cached("DELETE FROM chunks WHERE file_id = ?1")?
        .execute([file_id])?;
    Ok(())
}

fn replace_ignored(
    transaction: &Transaction<'_>,
    repository_id: &str,
    ignored: &[IgnoredPath],
    detected_at: &str,
) -> rusqlite::Result<()> {
    transaction.execute(
        "DELETE FROM ignored_paths WHERE repository_id = ?1",
        [repository_id],
    )?;
    for entry in ignored {
        transaction.execute(
            "INSERT INTO ignored_paths
             (id, repository_id, relative_path, entry_kind, reason, detected_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(repository_id, relative_path) DO UPDATE SET
               entry_kind = excluded.entry_kind,
               reason = excluded.reason,
               detected_at = excluded.detected_at",
            params![
                Uuid::now_v7().to_string(),
                repository_id,
                entry.relative_path,
                if entry.is_directory {
                    "directory"
                } else {
                    "file"
                },
                entry.reason.as_str(),
                detected_at,
            ],
        )?;
    }
    Ok(())
}

fn record_state(
    transaction: &Transaction<'_>,
    input: &PublishInput<'_>,
    updated_at: &str,
) -> rusqlite::Result<()> {
    transaction.execute(
        "INSERT INTO index_state
         (repository_id, head_oid, branch, history_head_oid, last_run_id, last_completed_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
         ON CONFLICT(repository_id) DO UPDATE SET
           head_oid = excluded.head_oid,
           branch = excluded.branch,
           history_head_oid = IFNULL(excluded.history_head_oid, index_state.history_head_oid),
           last_run_id = excluded.last_run_id,
           last_completed_at = excluded.last_completed_at,
           updated_at = excluded.updated_at",
        params![
            input.repository_id,
            input.head_oid,
            input.branch,
            input.history_head_oid,
            input.run_id,
            updated_at,
        ],
    )?;
    Ok(())
}

fn symbol_ids(
    transaction: &Transaction<'_>,
    file_id: &str,
) -> rusqlite::Result<HashMap<String, String>> {
    let mut statement =
        transaction.prepare_cached("SELECT stable_key, id FROM symbols WHERE file_id = ?1")?;
    statement
        .query_map([file_id], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect()
}

/// The symbol names a file currently contributes, read before it is replaced.
fn previous_names(
    transaction: &Transaction<'_>,
    file_id: &str,
) -> rusqlite::Result<Vec<(String, String)>> {
    let mut statement =
        transaction.prepare_cached("SELECT name, id FROM symbols WHERE file_id = ?1")?;
    statement
        .query_map([file_id], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect()
}

fn unique_target(
    transaction: &Transaction<'_>,
    repository_id: &str,
    target_key: &str,
) -> rusqlite::Result<Option<String>> {
    let mut statement = transaction.prepare_cached(
        "SELECT s.id FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE f.repository_id = ?1 AND f.deleted_at IS NULL AND s.name = ?2 LIMIT 2",
    )?;
    let matches: Vec<String> = statement
        .query_map(params![repository_id, target_key], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok((matches.len() == 1).then(|| matches[0].clone()))
}

/// Re-points the edges whose resolution this run could have changed.
///
/// An edge resolves only when its target name is unique among active symbols,
/// so adding or removing a definition can resolve or unresolve edges in other
/// files. Only the names this run disturbed are revisited, which keeps a
/// one-file re-index proportional to that file rather than to the repository.
fn reconcile_targets(
    transaction: &Transaction<'_>,
    repository_id: &str,
    touched: &Touched,
) -> rusqlite::Result<()> {
    // An edge pointing at a symbol this run removed loses its target.
    let mut orphan = transaction.prepare_cached(
        "UPDATE code_edges SET target_id = NULL, confidence = MIN(confidence, 0.45)
         WHERE repository_id = ?1 AND target_id = ?2",
    )?;
    for symbol_id in &touched.removed_symbol_ids {
        orphan.execute(params![repository_id, symbol_id])?;
    }
    drop(orphan);

    let mut resolve = transaction.prepare_cached(
        "UPDATE code_edges SET target_id = ?3, confidence = MAX(confidence, 0.9)
         WHERE repository_id = ?1 AND target_key = ?2 AND target_kind = 'symbol'",
    )?;
    let mut unresolve = transaction.prepare_cached(
        "UPDATE code_edges SET target_id = NULL, confidence = MIN(confidence, 0.45)
         WHERE repository_id = ?1 AND target_key = ?2 AND target_kind = 'symbol'
           AND target_id IS NOT NULL",
    )?;
    for name in &touched.names {
        match unique_target(transaction, repository_id, name)? {
            Some(target_id) => {
                resolve.execute(params![repository_id, name, target_id])?;
            }
            // A name that is no longer unique, or no longer exists, must stop
            // claiming a resolved target.
            None => {
                unresolve.execute(params![repository_id, name])?;
            }
        }
    }
    Ok(())
}
