use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use mars_storage::{Storage, StorageError};
use rusqlite::OptionalExtension;
use serde::Serialize;
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlapKind {
    ExactFile,
    Rename,
    ChangedSymbol,
    Dependency,
    UnknownIndex,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlapWarning {
    pub kind: OverlapKind,
    pub severity: String,
    pub message: String,
    pub evidence: Vec<String>,
    pub other_task_id: String,
    pub other_task_title: String,
    pub other_worktree_path: PathBuf,
    pub refreshed_at: String,
}

#[derive(Debug, Error)]
pub enum OverlapError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("Git overlap inspection failed: {0}")]
    Git(String),
    #[error("task `{0}` does not have a ready worktree")]
    TaskNotReady(String),
    #[error("failed to format overlap timestamp: {0}")]
    Timestamp(#[from] time::error::Format),
}

pub struct TaskOverlapService {
    storage: Arc<Storage>,
}

impl TaskOverlapService {
    #[must_use]
    pub fn new(storage: Arc<Storage>) -> Self {
        Self { storage }
    }

    pub fn warnings(&self, task_id: &str) -> Result<Vec<OverlapWarning>, OverlapError> {
        let contexts = self.contexts(task_id)?;
        let target_context = contexts
            .iter()
            .find(|context| context.task_id == task_id)
            .ok_or_else(|| OverlapError::TaskNotReady(task_id.into()))?;
        let target = self.changes(target_context)?;
        let dependencies = self.dependencies(&target_context.repository_id)?;
        let refreshed_at = OffsetDateTime::now_utc().format(&Rfc3339)?;
        let mut warnings = Vec::new();
        for context in contexts.iter().filter(|context| context.task_id != task_id) {
            let other = self.changes(context)?;
            warnings.extend(analyze(&target, &other, &dependencies, &refreshed_at));
        }
        warnings.sort_by(|left, right| {
            severity_rank(&right.severity)
                .cmp(&severity_rank(&left.severity))
                .then_with(|| left.other_task_title.cmp(&right.other_task_title))
                .then_with(|| left.message.cmp(&right.message))
        });
        warnings.dedup_by(|left, right| {
            left.kind == right.kind
                && left.other_task_id == right.other_task_id
                && left.evidence == right.evidence
        });
        Ok(warnings)
    }

    fn contexts(&self, task_id: &str) -> Result<Vec<TaskContext>, OverlapError> {
        let repository_id: Option<String> = self.storage.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT repository_id FROM tasks WHERE id = ?1",
                    [task_id],
                    |row| row.get(0),
                )
                .optional()
        })?;
        let repository_id =
            repository_id.ok_or_else(|| OverlapError::TaskNotReady(task_id.into()))?;
        Ok(self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT t.id, t.title, t.repository_id, w.path, b.base_ref
                 FROM tasks t
                 JOIN worktrees w ON w.task_id = t.id
                 JOIN branches b ON b.task_id = t.id
                 WHERE t.repository_id = ?1
                   AND t.status IN ('ready', 'active', 'review')
                   AND w.state = 'ready'
                 ORDER BY t.created_at, t.id",
            )?;
            statement
                .query_map([repository_id], |row| {
                    Ok(TaskContext {
                        task_id: row.get(0)?,
                        title: row.get(1)?,
                        repository_id: row.get(2)?,
                        worktree_path: PathBuf::from(row.get::<_, String>(3)?),
                        base_ref: row.get(4)?,
                    })
                })?
                .collect()
        })?)
    }

    fn changes(&self, context: &TaskContext) -> Result<TaskChanges, OverlapError> {
        let name_status = git(
            &context.worktree_path,
            &[
                "diff",
                "--name-status",
                "-z",
                "--find-renames",
                &context.base_ref,
            ],
        )?;
        let (paths, renames) = parse_name_status(&name_status);
        let hunks = parse_hunks(&git(
            &context.worktree_path,
            &["diff", "--unified=0", "--find-renames", &context.base_ref],
        )?);
        let index_complete: bool = self.storage.with_connection(|connection| {
            connection.query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM index_runs
                   WHERE repository_id = ?1 AND status = 'completed'
                 )",
                [&context.repository_id],
                |row| row.get(0),
            )
        })?;
        let symbols = self.storage.with_connection(|connection| {
            let mut symbols = BTreeMap::new();
            for path in &paths {
                let ranges = hunks.get(path);
                let mut statement = connection.prepare(
                    "SELECT s.id, s.name, s.start_line, s.end_line
                     FROM symbols s JOIN files f ON f.id = s.file_id
                     WHERE f.repository_id = ?1 AND f.relative_path = ?2 AND f.deleted_at IS NULL",
                )?;
                let records =
                    statement.query_map(rusqlite::params![context.repository_id, path], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, u32>(2)?,
                            row.get::<_, u32>(3)?,
                        ))
                    })?;
                for record in records {
                    let (id, name, start, end) = record?;
                    if ranges.is_none_or(|ranges| {
                        ranges.iter().any(|(changed_start, changed_end)| {
                            start <= *changed_end && end >= *changed_start
                        })
                    }) {
                        symbols.insert(id, format!("{name} ({path}:{start}-{end})"));
                    }
                }
            }
            Ok(symbols)
        })?;
        Ok(TaskChanges {
            task_id: context.task_id.clone(),
            title: context.title.clone(),
            worktree_path: context.worktree_path.clone(),
            paths,
            renames,
            symbols,
            index_complete,
        })
    }

    fn dependencies(&self, repository_id: &str) -> Result<HashSet<(String, String)>, OverlapError> {
        Ok(self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT source_id, target_id FROM code_edges
                 WHERE repository_id = ?1 AND source_kind = 'symbol' AND target_kind = 'symbol'
                   AND target_id IS NOT NULL",
            )?;
            statement
                .query_map([repository_id], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })?)
    }
}

struct TaskContext {
    task_id: String,
    title: String,
    repository_id: String,
    worktree_path: PathBuf,
    base_ref: String,
}

#[derive(Clone, Debug)]
struct TaskChanges {
    task_id: String,
    title: String,
    worktree_path: PathBuf,
    paths: BTreeSet<String>,
    renames: BTreeSet<(String, String)>,
    symbols: BTreeMap<String, String>,
    index_complete: bool,
}

fn analyze(
    target: &TaskChanges,
    other: &TaskChanges,
    dependencies: &HashSet<(String, String)>,
    refreshed_at: &str,
) -> Vec<OverlapWarning> {
    let mut warnings = Vec::new();
    for path in target.paths.intersection(&other.paths) {
        warnings.push(warning(
            OverlapKind::ExactFile,
            "error",
            format!("Both tasks change `{path}`"),
            vec![format!("file:{path}")],
            other,
            refreshed_at,
        ));
    }
    for (renames, counterpart_paths) in [
        (&target.renames, &other.paths),
        (&other.renames, &target.paths),
    ] {
        for (old, new) in renames {
            if !counterpart_paths.contains(old) && !counterpart_paths.contains(new) {
                continue;
            }
            warnings.push(warning(
                OverlapKind::Rename,
                "error",
                format!("A rename `{old}` → `{new}` intersects parallel work"),
                vec![format!("rename:{old}->{new}")],
                other,
                refreshed_at,
            ));
        }
    }
    for symbol_id in target
        .symbols
        .keys()
        .filter(|id| other.symbols.contains_key(*id))
    {
        warnings.push(warning(
            OverlapKind::ChangedSymbol,
            "warning",
            format!("Both tasks touch symbol `{}`", target.symbols[symbol_id]),
            vec![format!("symbol:{symbol_id}")],
            other,
            refreshed_at,
        ));
    }
    for (source, target_id) in dependencies {
        let crosses = (target.symbols.contains_key(source)
            && other.symbols.contains_key(target_id))
            || (target.symbols.contains_key(target_id) && other.symbols.contains_key(source));
        if crosses {
            warnings.push(warning(
                OverlapKind::Dependency,
                "warning",
                "Changed symbols are connected by a one-hop code dependency".into(),
                vec![format!("edge:{source}->{target_id}")],
                other,
                refreshed_at,
            ));
        }
    }
    if !target.index_complete || !other.index_complete {
        warnings.push(warning(
            OverlapKind::UnknownIndex,
            "info",
            "Symbol/dependency overlap is unknown because the repository index is incomplete"
                .into(),
            vec!["index:incomplete".into()],
            other,
            refreshed_at,
        ));
    }
    warnings
}

fn warning(
    kind: OverlapKind,
    severity: &str,
    message: String,
    evidence: Vec<String>,
    other: &TaskChanges,
    refreshed_at: &str,
) -> OverlapWarning {
    OverlapWarning {
        kind,
        severity: severity.into(),
        message,
        evidence,
        other_task_id: other.task_id.clone(),
        other_task_title: other.title.clone(),
        other_worktree_path: other.worktree_path.clone(),
        refreshed_at: refreshed_at.into(),
    }
}

fn parse_name_status(output: &str) -> (BTreeSet<String>, BTreeSet<(String, String)>) {
    let mut parts = output.split('\0').filter(|part| !part.is_empty());
    let mut paths = BTreeSet::new();
    let mut renames = BTreeSet::new();
    while let Some(status) = parts.next() {
        let Some(first) = parts.next() else { break };
        if status.starts_with('R') || status.starts_with('C') {
            let Some(second) = parts.next() else { break };
            paths.insert(first.into());
            paths.insert(second.into());
            renames.insert((first.into(), second.into()));
        } else {
            paths.insert(first.into());
        }
    }
    (paths, renames)
}

fn parse_hunks(diff: &str) -> BTreeMap<String, Vec<(u32, u32)>> {
    let mut path = None;
    let mut result: BTreeMap<String, Vec<(u32, u32)>> = BTreeMap::new();
    for line in diff.lines() {
        if let Some(next) = line.strip_prefix("+++ b/") {
            path = Some(next.to_owned());
        } else if line.starts_with("@@") {
            let Some(current) = path.as_ref() else {
                continue;
            };
            let Some(plus) = line.split_whitespace().find(|part| part.starts_with('+')) else {
                continue;
            };
            let mut bounds = plus.trim_start_matches('+').split(',');
            let Some(start) = bounds.next().and_then(|value| value.parse::<u32>().ok()) else {
                continue;
            };
            let count = bounds
                .next()
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(1);
            let end = start.saturating_add(count.saturating_sub(1));
            result
                .entry(current.clone())
                .or_default()
                .push((start, end));
        }
    }
    result
}

fn git(root: &Path, arguments: &[&str]) -> Result<String, OverlapError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .map_err(|error| OverlapError::Git(error.to_string()))?;
    if !output.status.success() {
        return Err(OverlapError::Git(
            String::from_utf8_lossy(&output.stderr).trim().into(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into())
}

fn severity_rank(severity: &str) -> u8 {
    match severity {
        "error" => 3,
        "warning" => 2,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changes(task: &str) -> TaskChanges {
        TaskChanges {
            task_id: task.into(),
            title: task.into(),
            worktree_path: PathBuf::from(format!("/tmp/{task}")),
            paths: BTreeSet::new(),
            renames: BTreeSet::new(),
            symbols: BTreeMap::new(),
            index_complete: true,
        }
    }

    #[test]
    fn detects_file_rename_symbol_dependency_and_unknown_overlap() {
        let mut target = changes("target");
        target
            .paths
            .extend(["src/a.ts".into(), "src/old.ts".into(), "src/new.ts".into()]);
        target
            .renames
            .insert(("src/old.ts".into(), "src/new.ts".into()));
        target.symbols.insert("symbol-a".into(), "alpha".into());
        target
            .symbols
            .insert("symbol-shared".into(), "shared".into());
        let mut other = changes("other");
        other
            .paths
            .extend(["src/a.ts".into(), "src/old.ts".into(), "src/b.ts".into()]);
        other.symbols.insert("symbol-b".into(), "beta".into());
        other
            .symbols
            .insert("symbol-shared".into(), "shared".into());
        other.index_complete = false;
        let dependencies = HashSet::from([("symbol-a".into(), "symbol-b".into())]);
        let warnings = analyze(&target, &other, &dependencies, "now");
        for kind in [
            OverlapKind::ExactFile,
            OverlapKind::Rename,
            OverlapKind::ChangedSymbol,
            OverlapKind::Dependency,
            OverlapKind::UnknownIndex,
        ] {
            assert!(
                warnings.iter().any(|warning| warning.kind == kind),
                "missing {kind:?}"
            );
        }
    }

    #[test]
    fn parses_rename_status_and_zero_context_hunks() {
        let (paths, renames) = parse_name_status("R100\0src/old.ts\0src/new.ts\0M\0src/a.ts\0");
        assert!(paths.contains("src/new.ts"));
        assert!(renames.contains(&("src/old.ts".into(), "src/new.ts".into())));
        let hunks = parse_hunks("+++ b/src/a.ts\n@@ -2 +4,3 @@\n+x\n");
        assert_eq!(hunks["src/a.ts"], vec![(4, 6)]);
    }
}
