use std::collections::HashMap;
use std::sync::Arc;

use mars_storage::Storage;
use rusqlite::{OptionalExtension, Transaction, params};
use uuid::Uuid;

use crate::IndexError;
use crate::javascript::ParsedFile;
use crate::timestamp;

pub fn publish(
    storage: &Arc<Storage>,
    repository_id: &str,
    run_id: &str,
    parsed: &[ParsedFile],
    deleted: &[String],
    files_seen: usize,
) -> Result<(), IndexError> {
    let indexed_at = timestamp()?;
    storage.with_transaction(|transaction| {
        for relative_path in deleted {
            retire_file(transaction, repository_id, relative_path, &indexed_at)?;
        }
        for file in parsed {
            replace_file(transaction, repository_id, run_id, file, &indexed_at)?;
        }
        reconcile_targets(transaction, repository_id)?;
        transaction.execute(
            "UPDATE index_runs SET status = 'completed', files_seen = ?2, finished_at = ?3 WHERE id = ?1",
            params![run_id, files_seen as i64, indexed_at],
        )?;
        Ok(())
    })?;
    Ok(())
}

fn replace_file(
    transaction: &Transaction<'_>,
    repository_id: &str,
    run_id: &str,
    parsed: &ParsedFile,
    indexed_at: &str,
) -> rusqlite::Result<()> {
    let existing_id: Option<String> = transaction
        .query_row(
            "SELECT id FROM files WHERE repository_id = ?1 AND relative_path = ?2",
            params![repository_id, parsed.source.relative_path],
            |row| row.get(0),
        )
        .optional()?;
    let file_id = existing_id.unwrap_or_else(|| Uuid::now_v7().to_string());
    let existing_symbols = symbol_ids(transaction, &file_id)?;

    for symbol_id in existing_symbols.values() {
        transaction.execute("DELETE FROM code_edges WHERE source_id = ?1", [symbol_id])?;
    }
    transaction.execute(
        "DELETE FROM code_edges WHERE source_kind = 'file' AND source_id = ?1",
        [&file_id],
    )?;
    transaction.execute("DELETE FROM symbols WHERE file_id = ?1", [&file_id])?;
    transaction.execute(
        "INSERT INTO files
         (id, repository_id, relative_path, language, content_hash, byte_size, mtime_ns, parse_errors, indexed_at, deleted_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL)
         ON CONFLICT(repository_id, relative_path) DO UPDATE SET
           language = excluded.language,
           content_hash = excluded.content_hash,
           byte_size = excluded.byte_size,
           mtime_ns = excluded.mtime_ns,
           parse_errors = excluded.parse_errors,
           indexed_at = excluded.indexed_at,
           deleted_at = NULL",
        params![
            file_id,
            repository_id,
            parsed.source.relative_path,
            parsed.source.language,
            parsed.source.content_hash,
            parsed.source.byte_size as i64,
            parsed.source.mtime_ns,
            parsed.parse_errors as i64,
            indexed_at,
        ],
    )?;

    let mut symbol_by_key = HashMap::new();
    for symbol in &parsed.symbols {
        let symbol_id = existing_symbols
            .get(&symbol.stable_key)
            .cloned()
            .unwrap_or_else(|| Uuid::now_v7().to_string());
        transaction.execute(
            "INSERT INTO symbols
             (id, file_id, stable_key, kind, name, qualified_name, start_byte, end_byte, start_line, end_line, signature, exported, content_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
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
            ],
        )?;
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
        transaction.execute(
            "INSERT OR REPLACE INTO code_edges
             (id, repository_id, source_kind, source_id, edge_kind, target_kind, target_id, target_key, confidence, evidence_json, index_run_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
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
            ],
        )?;
    }
    Ok(())
}

fn retire_file(
    transaction: &Transaction<'_>,
    repository_id: &str,
    relative_path: &str,
    deleted_at: &str,
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
    let symbols = symbol_ids(transaction, &file_id)?;
    for symbol_id in symbols.values() {
        transaction.execute(
            "DELETE FROM code_edges WHERE source_id = ?1 OR target_id = ?1",
            [symbol_id],
        )?;
    }
    transaction.execute(
        "DELETE FROM code_edges WHERE source_kind = 'file' AND source_id = ?1",
        [&file_id],
    )?;
    transaction.execute("DELETE FROM symbols WHERE file_id = ?1", [&file_id])?;
    transaction.execute(
        "UPDATE files SET deleted_at = ?2 WHERE id = ?1",
        params![file_id, deleted_at],
    )?;
    Ok(())
}

fn symbol_ids(
    transaction: &Transaction<'_>,
    file_id: &str,
) -> rusqlite::Result<HashMap<String, String>> {
    let mut statement =
        transaction.prepare("SELECT stable_key, id FROM symbols WHERE file_id = ?1")?;
    statement
        .query_map([file_id], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect()
}

fn unique_target(
    transaction: &Transaction<'_>,
    repository_id: &str,
    target_key: &str,
) -> rusqlite::Result<Option<String>> {
    let mut statement = transaction.prepare(
        "SELECT s.id FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE f.repository_id = ?1 AND f.deleted_at IS NULL AND s.name = ?2 LIMIT 2",
    )?;
    let matches: Vec<String> = statement
        .query_map(params![repository_id, target_key], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok((matches.len() == 1).then(|| matches[0].clone()))
}

fn reconcile_targets(transaction: &Transaction<'_>, repository_id: &str) -> rusqlite::Result<()> {
    transaction.execute(
        "UPDATE code_edges SET target_id = NULL, confidence = MIN(confidence, 0.45)
         WHERE repository_id = ?1 AND target_id IS NOT NULL
           AND target_id NOT IN (SELECT id FROM symbols)",
        [repository_id],
    )?;
    let mut statement = transaction.prepare(
        "SELECT DISTINCT target_key FROM code_edges WHERE repository_id = ?1 AND target_kind = 'symbol'",
    )?;
    let keys: Vec<String> = statement
        .query_map([repository_id], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    drop(statement);
    for key in keys {
        if let Some(target_id) = unique_target(transaction, repository_id, &key)? {
            transaction.execute(
                "UPDATE code_edges SET target_id = ?3, confidence = MAX(confidence, 0.9)
                 WHERE repository_id = ?1 AND target_key = ?2 AND target_kind = 'symbol'",
                params![repository_id, key, target_id],
            )?;
        }
    }
    Ok(())
}
