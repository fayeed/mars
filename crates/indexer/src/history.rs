//! The Git history layer: commits, per-file churn, last-touched-by, and blame.
//!
//! History is read through the system Git binary, matching ADR-004. Blame is
//! computed on demand against the working tree rather than being cached, so a
//! cited line range is never attributed from a stale snapshot.

use std::path::Path;
use std::sync::Arc;

use mars_git::command::{self, GitCommandError};
use mars_storage::Storage;
use rusqlite::params;
use serde::Serialize;
use uuid::Uuid;

use crate::IndexError;

/// How far back a cold history ingest reaches. Later runs are incremental.
pub const DEFAULT_HISTORY_DEPTH: usize = 2000;

const RECORD_SEPARATOR: char = '\u{1e}';
const FIELD_SEPARATOR: char = '\u{1f}';

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistorySummary {
    pub commits_ingested: usize,
    pub head_oid: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlameRange {
    pub commit_oid: String,
    pub author_name: String,
    pub author_email: String,
    pub authored_at: String,
    pub summary: String,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChurn {
    pub relative_path: String,
    pub commits: usize,
    pub insertions: usize,
    pub deletions: usize,
    pub last_commit_oid: Option<String>,
    pub last_touched_by: Option<String>,
    pub last_touched_at: Option<String>,
}

#[must_use]
pub fn head_oid(root: &Path) -> Option<String> {
    command::run(root, ["rev-parse", "HEAD"])
        .ok()
        .map(|output| output.stdout_text())
        .filter(|oid| !oid.is_empty())
}

#[must_use]
pub fn current_branch(root: &Path) -> Option<String> {
    command::run(root, ["rev-parse", "--abbrev-ref", "HEAD"])
        .ok()
        .map(|output| output.stdout_text())
        .filter(|branch| !branch.is_empty() && branch != "HEAD")
}

/// Ingests commits reachable from HEAD. When `since` names an already-ingested
/// commit, only the commits added since then are read.
pub fn ingest(
    storage: &Arc<Storage>,
    repository_id: &str,
    root: &Path,
    since: Option<&str>,
    depth: usize,
) -> Result<HistorySummary, IndexError> {
    let head = head_oid(root);
    let Some(head) = head else {
        // A repository with no commits yet has no history to ingest.
        return Ok(HistorySummary::default());
    };
    if since.is_some_and(|since| since == head) {
        return Ok(HistorySummary {
            commits_ingested: 0,
            head_oid: Some(head),
        });
    }

    let range = since.map(|since| format!("{since}..HEAD"));
    let depth = format!("--max-count={depth}");
    let mut arguments = vec![
        "log",
        "--raw",
        "--numstat",
        "-M",
        "-z",
        "--no-color",
        &depth,
        "--format=%x1e%H%x1f%P%x1f%an%x1f%ae%x1f%aI%x1f%cI%x1f%s",
    ];
    if let Some(range) = range.as_deref() {
        arguments.push(range);
    }

    let output = match command::run(root, arguments) {
        Ok(output) => output,
        // An unknown `since` (history rewritten, or a fresh clone) is recoverable
        // by falling back to a bounded full read.
        Err(GitCommandError::Failed { .. }) if since.is_some() => {
            return ingest(storage, repository_id, root, None, depth_value(&depth));
        }
        Err(error) => return Err(IndexError::Git(error.to_string())),
    };

    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let commits = parse_log(&text);
    let ingested = commits.len();

    storage.with_transaction(|transaction| {
        for commit in &commits {
            let commit_id = Uuid::now_v7().to_string();
            let inserted = transaction.execute(
                "INSERT INTO commits
                 (id, repository_id, oid, parent_oids, author_name, author_email, authored_at, committed_at, subject)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(repository_id, oid) DO NOTHING",
                params![
                    commit_id,
                    repository_id,
                    commit.oid,
                    commit.parent_oids,
                    commit.author_name,
                    commit.author_email,
                    commit.authored_at,
                    commit.committed_at,
                    commit.subject,
                ],
            )?;
            if inserted == 0 {
                continue;
            }
            for change in &commit.changes {
                transaction.execute(
                    "INSERT INTO commit_files
                     (id, repository_id, commit_id, relative_path, previous_path, change_kind, insertions, deletions)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT(commit_id, relative_path) DO NOTHING",
                    params![
                        Uuid::now_v7().to_string(),
                        repository_id,
                        commit_id,
                        change.relative_path,
                        change.previous_path,
                        change.change_kind,
                        change.insertions as i64,
                        change.deletions as i64,
                    ],
                )?;
            }
        }
        Ok(())
    })?;

    Ok(HistorySummary {
        commits_ingested: ingested,
        head_oid: Some(head),
    })
}

fn depth_value(flag: &str) -> usize {
    flag.rsplit('=')
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_HISTORY_DEPTH)
}

#[derive(Debug)]
struct CommitRecord {
    oid: String,
    parent_oids: String,
    author_name: String,
    author_email: String,
    authored_at: String,
    committed_at: String,
    subject: String,
    changes: Vec<ChangeRecord>,
}

#[derive(Debug)]
struct ChangeRecord {
    relative_path: String,
    previous_path: Option<String>,
    change_kind: String,
    insertions: usize,
    deletions: usize,
}

/// Parses `git log --raw --numstat -z`: a raw block carrying change status,
/// followed by a numstat block carrying line counts, per commit.
fn parse_log(text: &str) -> Vec<CommitRecord> {
    let mut commits = Vec::new();
    for record in text.split(RECORD_SEPARATOR).skip(1) {
        let mut tokens = record.split('\0');
        let Some(header) = tokens.next() else {
            continue;
        };
        let fields: Vec<&str> = header.trim_start_matches('\n').split(FIELD_SEPARATOR).collect();
        if fields.len() < 7 {
            continue;
        }
        let mut changes: Vec<ChangeRecord> = Vec::new();
        let tokens: Vec<&str> = tokens.collect();
        let mut index = 0;
        while index < tokens.len() {
            let token = tokens[index].trim_start_matches('\n');
            if token.is_empty() {
                index += 1;
                continue;
            }
            if token.starts_with(':') {
                let status = token.rsplit(' ').next().unwrap_or("M");
                let renamed = status.starts_with('R') || status.starts_with('C');
                let (previous_path, path) = if renamed {
                    let previous = tokens.get(index + 1).map(|value| (*value).to_owned());
                    let path = tokens.get(index + 2).unwrap_or(&"").to_string();
                    index += 3;
                    (previous, path)
                } else {
                    let path = tokens.get(index + 1).unwrap_or(&"").to_string();
                    index += 2;
                    (None, path)
                };
                if path.is_empty() {
                    continue;
                }
                changes.push(ChangeRecord {
                    relative_path: path,
                    previous_path,
                    change_kind: change_kind(status).to_owned(),
                    insertions: 0,
                    deletions: 0,
                });
                continue;
            }

            let mut parts = token.splitn(3, '\t');
            let insertions = parts.next().and_then(|value| value.parse().ok()).unwrap_or(0);
            let deletions = parts.next().and_then(|value| value.parse().ok()).unwrap_or(0);
            let path = parts.next().unwrap_or("");
            let path = if path.is_empty() {
                // A rename in numstat puts the old and new paths in the next tokens.
                let renamed = tokens.get(index + 2).unwrap_or(&"").to_string();
                index += 3;
                renamed
            } else {
                index += 1;
                path.to_owned()
            };
            if let Some(change) = changes
                .iter_mut()
                .find(|change| change.relative_path == path)
            {
                change.insertions = insertions;
                change.deletions = deletions;
            } else if !path.is_empty() {
                changes.push(ChangeRecord {
                    relative_path: path,
                    previous_path: None,
                    change_kind: "modified".into(),
                    insertions,
                    deletions,
                });
            }
        }

        commits.push(CommitRecord {
            oid: fields[0].to_owned(),
            parent_oids: fields[1].to_owned(),
            author_name: fields[2].to_owned(),
            author_email: fields[3].to_owned(),
            authored_at: fields[4].to_owned(),
            committed_at: fields[5].to_owned(),
            subject: fields[6].to_owned(),
            changes,
        });
    }
    commits
}

fn change_kind(status: &str) -> &'static str {
    match status.chars().next() {
        Some('A') => "added",
        Some('D') => "deleted",
        Some('R') => "renamed",
        Some('C') => "copied",
        Some('T') => "type-changed",
        _ => "modified",
    }
}

/// Blames a line range against the working tree.
pub fn blame(
    root: &Path,
    relative_path: &str,
    start_line: usize,
    end_line: usize,
) -> Result<Vec<BlameRange>, IndexError> {
    let start = start_line.max(1);
    let end = end_line.max(start);
    let range = format!("{start},{end}");
    let output = command::run(
        root,
        [
            "blame",
            "--porcelain",
            "-L",
            &range,
            "--",
            relative_path,
        ],
    )
    .map_err(|error| IndexError::Git(error.to_string()))?;
    Ok(parse_blame(&String::from_utf8_lossy(&output.stdout)))
}

/// Parses `git blame --porcelain`, coalescing consecutive lines from the same
/// commit into one range.
fn parse_blame(text: &str) -> Vec<BlameRange> {
    let mut ranges: Vec<BlameRange> = Vec::new();
    let mut authors: std::collections::HashMap<String, (String, String, String, String)> =
        std::collections::HashMap::new();
    let mut current_oid = String::new();
    let mut current_line = 0_usize;
    let mut pending: Option<(String, String, String, String)> = None;

    for line in text.lines() {
        if line.starts_with('\t') {
            let details = pending
                .clone()
                .or_else(|| authors.get(&current_oid).cloned())
                .unwrap_or_default();
            authors.insert(current_oid.clone(), details.clone());
            pending = None;
            match ranges.last_mut() {
                Some(last) if last.commit_oid == current_oid && last.end_line + 1 == current_line => {
                    last.end_line = current_line;
                }
                _ => ranges.push(BlameRange {
                    commit_oid: current_oid.clone(),
                    author_name: details.0,
                    author_email: details.1,
                    authored_at: details.2,
                    summary: details.3,
                    start_line: current_line,
                    end_line: current_line,
                }),
            }
            continue;
        }

        if let Some((key, value)) = line.split_once(' ') {
            match key {
                "author" => entry(&mut pending).0 = value.to_owned(),
                "author-mail" => {
                    entry(&mut pending).1 = value.trim_matches(['<', '>']).to_owned();
                }
                "author-time" => entry(&mut pending).2 = value.to_owned(),
                "summary" => entry(&mut pending).3 = value.to_owned(),
                _ => {
                    // A header line is `<oid> <orig-line> <final-line> [count]`.
                    if key.len() == 40 && key.chars().all(|character| character.is_ascii_hexdigit())
                    {
                        current_oid = key.to_owned();
                        current_line = value
                            .split(' ')
                            .nth(1)
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(current_line + 1);
                    }
                }
            }
        }
    }
    ranges
}

fn entry(pending: &mut Option<(String, String, String, String)>) -> &mut (String, String, String, String) {
    pending.get_or_insert_with(Default::default)
}

/// Reads churn and last-touched-by for a path from the ingested history.
pub fn churn(
    storage: &Arc<Storage>,
    repository_id: &str,
    relative_path: &str,
) -> Result<FileChurn, IndexError> {
    let totals: (i64, i64, i64) = storage.with_connection(|connection| {
        connection.query_row(
            "SELECT COUNT(*), IFNULL(SUM(insertions), 0), IFNULL(SUM(deletions), 0)
             FROM commit_files
             WHERE repository_id = ?1 AND relative_path = ?2",
            params![repository_id, relative_path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
    })?;

    let last: Option<(String, String, String)> = storage.with_connection(|connection| {
        connection
            .query_row(
                "SELECT c.oid, c.author_name, c.committed_at
                 FROM commit_files f JOIN commits c ON c.id = f.commit_id
                 WHERE f.repository_id = ?1 AND f.relative_path = ?2
                 ORDER BY c.committed_at DESC LIMIT 1",
                params![repository_id, relative_path],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .ok()
            .map_or(Ok(None), |value| Ok(Some(value)))
    })?;

    Ok(FileChurn {
        relative_path: relative_path.to_owned(),
        commits: usize::try_from(totals.0).unwrap_or(0),
        insertions: usize::try_from(totals.1).unwrap_or(0),
        deletions: usize::try_from(totals.2).unwrap_or(0),
        last_commit_oid: last.as_ref().map(|last| last.0.clone()),
        last_touched_by: last.as_ref().map(|last| last.1.clone()),
        last_touched_at: last.map(|last| last.2),
    })
}
