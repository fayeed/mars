use std::sync::Arc;

use mars_storage::Storage;
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::{CheckRun, ForgeError, ForgeRepository, Issue, PullRequest, ReviewComment};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteRecord {
    pub id: String,
    pub repository_id: String,
    pub name: String,
    pub fetch_url: String,
    pub provider: String,
    pub repository: ForgeRepository,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgeConnection {
    pub id: String,
    pub remote_id: String,
    pub provider: String,
    pub api_base: String,
    pub account_login: Option<String>,
    pub credential_ref: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredIssue {
    pub id: String,
    pub remote_id: String,
    pub repository_id: String,
    #[serde(flatten)]
    pub issue: Issue,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestSnapshot {
    pub id: String,
    pub remote_id: String,
    pub task_id: Option<String>,
    pub issue_id: Option<String>,
    #[serde(flatten)]
    pub pull_request: PullRequest,
    pub comments: Vec<ReviewComment>,
    pub checks: Vec<CheckRun>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgeMutation {
    pub id: String,
    pub task_id: String,
    pub remote_id: String,
    pub operation: String,
    pub status: String,
    pub plan_fingerprint: String,
    pub push_approved: bool,
    pub forge_approved: bool,
    pub details_json: String,
    pub error: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
}

pub struct ForgeStore {
    storage: Arc<Storage>,
}

impl ForgeStore {
    #[must_use]
    pub fn new(storage: Arc<Storage>) -> Self {
        Self { storage }
    }

    pub fn upsert_remote(
        &self,
        repository_id: &str,
        name: &str,
        fetch_url: &str,
        provider: &str,
        repository: &ForgeRepository,
    ) -> Result<RemoteRecord, ForgeError> {
        let now = now()?;
        let id = self.storage.with_transaction(|transaction| {
            let existing: Option<String> = transaction
                .query_row(
                    "SELECT id FROM remotes WHERE repository_id = ?1 AND name = ?2",
                    params![repository_id, name],
                    |row| row.get(0),
                )
                .optional()?;
            let id = existing.unwrap_or_else(|| Uuid::now_v7().to_string());
            transaction.execute(
                "INSERT INTO remotes
                 (id, repository_id, name, fetch_url, provider, host, owner, repository_name, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
                 ON CONFLICT(repository_id, name) DO UPDATE SET
                   fetch_url = excluded.fetch_url, provider = excluded.provider, host = excluded.host,
                   owner = excluded.owner, repository_name = excluded.repository_name,
                   updated_at = excluded.updated_at",
                params![
                    id,
                    repository_id,
                    name,
                    fetch_url,
                    provider,
                    repository.host,
                    repository.owner,
                    repository.name,
                    now
                ],
            )?;
            Ok(id)
        })?;
        Ok(RemoteRecord {
            id,
            repository_id: repository_id.into(),
            name: name.into(),
            fetch_url: fetch_url.into(),
            provider: provider.into(),
            repository: repository.clone(),
        })
    }

    pub fn upsert_connection(
        &self,
        remote_id: &str,
        provider: &str,
        api_base: &str,
        account_login: Option<&str>,
        credential_ref: &str,
    ) -> Result<ForgeConnection, ForgeError> {
        let now = now()?;
        let id = self.storage.with_transaction(|transaction| {
            transaction.execute(
                "UPDATE remotes SET provider = ?2, updated_at = ?3 WHERE id = ?1",
                params![remote_id, provider, now],
            )?;
            let existing: Option<String> = transaction
                .query_row(
                    "SELECT id FROM forge_connections WHERE remote_id = ?1",
                    [remote_id],
                    |row| row.get(0),
                )
                .optional()?;
            let id = existing.unwrap_or_else(|| Uuid::now_v7().to_string());
            transaction.execute(
                "INSERT INTO forge_connections
                 (id, remote_id, provider, api_base, account_login, credential_ref, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
                 ON CONFLICT(remote_id) DO UPDATE SET
                   provider = excluded.provider, api_base = excluded.api_base,
                   account_login = excluded.account_login, credential_ref = excluded.credential_ref,
                   updated_at = excluded.updated_at",
                params![id, remote_id, provider, api_base, account_login, credential_ref, now],
            )?;
            Ok(id)
        })?;
        Ok(ForgeConnection {
            id,
            remote_id: remote_id.into(),
            provider: provider.into(),
            api_base: api_base.into(),
            account_login: account_login.map(Into::into),
            credential_ref: credential_ref.into(),
        })
    }

    pub fn get_connection(&self, remote_id: &str) -> Result<Option<ForgeConnection>, ForgeError> {
        Ok(self.storage.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT id, remote_id, provider, api_base, account_login, credential_ref
                     FROM forge_connections WHERE remote_id = ?1",
                    [remote_id],
                    |row| {
                        Ok(ForgeConnection {
                            id: row.get(0)?,
                            remote_id: row.get(1)?,
                            provider: row.get(2)?,
                            api_base: row.get(3)?,
                            account_login: row.get(4)?,
                            credential_ref: row.get(5)?,
                        })
                    },
                )
                .optional()
        })?)
    }

    pub fn start_mutation(
        &self,
        task_id: &str,
        remote_id: &str,
        plan_fingerprint: &str,
        push_approved: bool,
        forge_approved: bool,
        details_json: &str,
    ) -> Result<String, ForgeError> {
        let id = Uuid::now_v7().to_string();
        let started_at = now()?;
        self.storage.with_connection(|connection| {
            connection.execute(
                "INSERT INTO forge_mutations
                 (id, task_id, remote_id, operation, status, plan_fingerprint, push_approved,
                  forge_approved, details_json, error, started_at, finished_at)
                 VALUES (?1, ?2, ?3, 'create_pull_request', 'started', ?4, ?5, ?6, ?7, NULL, ?8, NULL)",
                params![
                    id,
                    task_id,
                    remote_id,
                    plan_fingerprint,
                    push_approved,
                    forge_approved,
                    details_json,
                    started_at
                ],
            )?;
            Ok(())
        })?;
        Ok(id)
    }

    pub fn finish_mutation(
        &self,
        id: &str,
        status: &str,
        details_json: &str,
        error: Option<&str>,
    ) -> Result<(), ForgeError> {
        if !matches!(status, "completed" | "failed") {
            return Err(ForgeError::InvalidResponse(format!(
                "invalid terminal forge mutation status `{status}`"
            )));
        }
        let finished_at = now()?;
        let changed = self.storage.with_connection(|connection| {
            connection.execute(
                "UPDATE forge_mutations
                 SET status = ?2, details_json = ?3, error = ?4, finished_at = ?5
                 WHERE id = ?1 AND status = 'started'",
                params![id, status, details_json, error, finished_at],
            )
        })?;
        if changed != 1 {
            return Err(ForgeError::InvalidResponse(format!(
                "active forge mutation `{id}` was not found"
            )));
        }
        Ok(())
    }

    pub fn recover_interrupted_mutations(&self) -> Result<usize, ForgeError> {
        let finished_at = now()?;
        let changed = self.storage.with_connection(|connection| {
            connection.execute(
                "UPDATE forge_mutations
                 SET status = 'interrupted', error = 'application stopped before confirmation',
                     finished_at = ?1
                 WHERE status = 'started'",
                [finished_at],
            )
        })?;
        Ok(changed)
    }

    pub fn latest_mutation(&self, task_id: &str) -> Result<Option<ForgeMutation>, ForgeError> {
        Ok(self.storage.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT id, task_id, remote_id, operation, status, plan_fingerprint,
                            push_approved, forge_approved, details_json, error, started_at, finished_at
                     FROM forge_mutations WHERE task_id = ?1
                     ORDER BY started_at DESC, id DESC LIMIT 1",
                    [task_id],
                    |row| {
                        Ok(ForgeMutation {
                            id: row.get(0)?,
                            task_id: row.get(1)?,
                            remote_id: row.get(2)?,
                            operation: row.get(3)?,
                            status: row.get(4)?,
                            plan_fingerprint: row.get(5)?,
                            push_approved: row.get(6)?,
                            forge_approved: row.get(7)?,
                            details_json: row.get(8)?,
                            error: row.get(9)?,
                            started_at: row.get(10)?,
                            finished_at: row.get(11)?,
                        })
                    },
                )
                .optional()
        })?)
    }

    pub fn upsert_issue(&self, remote_id: &str, issue: &Issue) -> Result<StoredIssue, ForgeError> {
        let now = now()?;
        let labels = serde_json::to_string(&issue.labels)?;
        let number = sql_integer(issue.number, "issue number")?;
        let (id, repository_id) = self.storage.with_transaction(|transaction| {
            let repository_id: String = transaction.query_row(
                "SELECT repository_id FROM remotes WHERE id = ?1",
                [remote_id],
                |row| row.get(0),
            )?;
            let existing: Option<String> = transaction
                .query_row(
                    "SELECT id FROM issues WHERE remote_id = ?1 AND provider_id = ?2",
                    params![remote_id, issue.provider_id],
                    |row| row.get(0),
                )
                .optional()?;
            let id = existing.unwrap_or_else(|| Uuid::now_v7().to_string());
            transaction.execute(
                "INSERT INTO issues
                 (id, remote_id, provider_id, number, title, body, state, url, author, labels_json,
                  provider_updated_at, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)
                 ON CONFLICT(remote_id, provider_id) DO UPDATE SET
                   number = excluded.number, title = excluded.title, body = excluded.body,
                   state = excluded.state, url = excluded.url, author = excluded.author,
                   labels_json = excluded.labels_json, provider_updated_at = excluded.provider_updated_at,
                   updated_at = excluded.updated_at",
                params![
                    id,
                    remote_id,
                    issue.provider_id,
                    number,
                    issue.title,
                    issue.body,
                    issue.state,
                    issue.url,
                    issue.author,
                    labels,
                    issue.updated_at,
                    now
                ],
            )?;
            Ok((id, repository_id))
        })?;
        Ok(StoredIssue {
            id,
            remote_id: remote_id.into(),
            repository_id,
            issue: issue.clone(),
        })
    }

    pub fn get_remote(&self, remote_id: &str) -> Result<RemoteRecord, ForgeError> {
        self.storage
            .with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT id, repository_id, name, fetch_url, provider, host, owner, repository_name
                         FROM remotes WHERE id = ?1",
                        [remote_id],
                        |row| {
                            Ok(RemoteRecord {
                                id: row.get(0)?,
                                repository_id: row.get(1)?,
                                name: row.get(2)?,
                                fetch_url: row.get(3)?,
                                provider: row.get(4)?,
                                repository: ForgeRepository {
                                    host: row.get(5)?,
                                    owner: row.get(6)?,
                                    name: row.get(7)?,
                                },
                            })
                        },
                    )
                    .optional()
            })?
            .ok_or_else(|| ForgeError::InvalidResponse("stored remote was not found".into()))
    }

    pub fn get_issue(&self, issue_id: &str) -> Result<StoredIssue, ForgeError> {
        let stored = self.storage.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT i.id, i.remote_id, r.repository_id, i.provider_id, i.number, i.title,
                            i.body, i.state, i.url, i.author, i.labels_json, i.provider_updated_at
                     FROM issues i
                     JOIN remotes r ON r.id = i.remote_id
                     WHERE i.id = ?1",
                    [issue_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, i64>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, String>(6)?,
                            row.get::<_, String>(7)?,
                            row.get::<_, String>(8)?,
                            row.get::<_, String>(9)?,
                            row.get::<_, String>(10)?,
                            row.get::<_, String>(11)?,
                        ))
                    },
                )
                .optional()
        })?;
        let (
            id,
            remote_id,
            repository_id,
            provider_id,
            number,
            title,
            body,
            state,
            url,
            author,
            labels,
            updated_at,
        ) = stored
            .ok_or_else(|| ForgeError::InvalidResponse("stored issue was not found".into()))?;
        Ok(StoredIssue {
            id,
            remote_id,
            repository_id,
            issue: Issue {
                provider_id,
                number: u64::try_from(number).map_err(|_| {
                    ForgeError::InvalidResponse("stored issue number was invalid".into())
                })?,
                title,
                body,
                state,
                url,
                author,
                labels: serde_json::from_str(&labels)?,
                updated_at,
            },
        })
    }

    pub fn link_issue_task(&self, issue_id: &str, task_id: &str) -> Result<(), ForgeError> {
        let now = now()?;
        self.storage.with_connection(|connection| {
            connection.execute(
                "INSERT INTO issue_task_links (issue_id, task_id, created_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(task_id) DO UPDATE SET issue_id = excluded.issue_id",
                params![issue_id, task_id, now],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    pub fn record_pull_request(
        &self,
        remote_id: &str,
        task_id: Option<&str>,
        issue_id: Option<&str>,
        pull_request: &PullRequest,
        comments: &[ReviewComment],
        checks: &[CheckRun],
    ) -> Result<PullRequestSnapshot, ForgeError> {
        let now = now()?;
        let number = sql_integer(pull_request.number, "pull request number")?;
        let comment_lines = comments
            .iter()
            .map(|comment| {
                comment
                    .line
                    .map(|line| sql_integer(line, "review comment line"))
                    .transpose()
            })
            .collect::<Result<Vec<_>, _>>()?;
        let id = self.storage.with_transaction(|transaction| {
            let existing: Option<String> = transaction
                .query_row(
                    "SELECT id FROM pull_requests WHERE remote_id = ?1 AND provider_id = ?2",
                    params![remote_id, pull_request.provider_id],
                    |row| row.get(0),
                )
                .optional()?;
            let id = existing.unwrap_or_else(|| Uuid::now_v7().to_string());
            transaction.execute(
                "INSERT INTO pull_requests
                 (id, remote_id, task_id, issue_id, provider_id, number, title, body, state, url,
                  head, base, draft, mergeable, head_sha, provider_updated_at, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?17)
                 ON CONFLICT(remote_id, provider_id) DO UPDATE SET
                   task_id = COALESCE(excluded.task_id, pull_requests.task_id),
                   issue_id = COALESCE(excluded.issue_id, pull_requests.issue_id),
                   title = excluded.title, body = excluded.body, state = excluded.state, url = excluded.url,
                   head = excluded.head, base = excluded.base, draft = excluded.draft,
                   mergeable = excluded.mergeable, head_sha = excluded.head_sha,
                   provider_updated_at = excluded.provider_updated_at, updated_at = excluded.updated_at",
                params![
                    id,
                    remote_id,
                    task_id,
                    issue_id,
                    pull_request.provider_id,
                    number,
                    pull_request.title,
                    pull_request.body,
                    pull_request.state,
                    pull_request.url,
                    pull_request.head,
                    pull_request.base,
                    pull_request.draft,
                    pull_request.mergeable,
                    pull_request.head_sha,
                    pull_request.updated_at,
                    now
                ],
            )?;
            for (comment, line) in comments.iter().zip(&comment_lines) {
                transaction.execute(
                    "INSERT INTO review_comments
                     (id, pull_request_id, provider_id, author, body, relative_path, line, url,
                      provider_created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                     ON CONFLICT(pull_request_id, provider_id) DO UPDATE SET
                       author = excluded.author, body = excluded.body, relative_path = excluded.relative_path,
                       line = excluded.line, url = excluded.url, updated_at = excluded.updated_at",
                    params![
                        Uuid::now_v7().to_string(), id, comment.provider_id, comment.author,
                        comment.body, comment.path, line, comment.url, comment.created_at, now
                    ],
                )?;
            }
            for check in checks {
                transaction.execute(
                    "INSERT INTO check_runs
                     (id, pull_request_id, provider_id, name, status, conclusion, url, started_at,
                      completed_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                     ON CONFLICT(pull_request_id, provider_id) DO UPDATE SET
                       name = excluded.name, status = excluded.status, conclusion = excluded.conclusion,
                       url = excluded.url, started_at = excluded.started_at,
                       completed_at = excluded.completed_at, updated_at = excluded.updated_at",
                    params![
                        Uuid::now_v7().to_string(), id, check.provider_id, check.name, check.status,
                        check.conclusion, check.url, check.started_at, check.completed_at, now
                    ],
                )?;
            }
            Ok(id)
        })?;
        Ok(PullRequestSnapshot {
            id,
            remote_id: remote_id.into(),
            task_id: task_id.map(Into::into),
            issue_id: issue_id.map(Into::into),
            pull_request: pull_request.clone(),
            comments: comments.to_vec(),
            checks: checks.to_vec(),
        })
    }

    pub fn get_task_pull_request(
        &self,
        task_id: &str,
    ) -> Result<Option<PullRequestSnapshot>, ForgeError> {
        let stored = self.storage.with_connection(|connection| {
            let row = connection
                .query_row(
                    "SELECT id, remote_id, task_id, issue_id, provider_id, number, title, body,
                            state, url, head, base, draft, mergeable, head_sha, provider_updated_at
                     FROM pull_requests
                     WHERE task_id = ?1
                     ORDER BY updated_at DESC, id DESC
                     LIMIT 1",
                    [task_id],
                    |row| {
                        Ok(PullRequestRow {
                            id: row.get(0)?,
                            remote_id: row.get(1)?,
                            task_id: row.get(2)?,
                            issue_id: row.get(3)?,
                            provider_id: row.get(4)?,
                            number: row.get(5)?,
                            title: row.get(6)?,
                            body: row.get(7)?,
                            state: row.get(8)?,
                            url: row.get(9)?,
                            head: row.get(10)?,
                            base: row.get(11)?,
                            draft: row.get(12)?,
                            mergeable: row.get(13)?,
                            head_sha: row.get(14)?,
                            updated_at: row.get(15)?,
                        })
                    },
                )
                .optional()?;
            let Some(row) = row else {
                return Ok(None);
            };
            let comments = connection
                .prepare(
                    "SELECT provider_id, author, body, relative_path, line, url, provider_created_at
                     FROM review_comments WHERE pull_request_id = ?1
                     ORDER BY provider_created_at, id",
                )?
                .query_map([&row.id], |row| {
                    Ok(ReviewComment {
                        provider_id: row.get(0)?,
                        author: row.get(1)?,
                        body: row.get(2)?,
                        path: row.get(3)?,
                        line: row
                            .get::<_, Option<i64>>(4)?
                            .and_then(|line| u64::try_from(line).ok()),
                        url: row.get(5)?,
                        created_at: row.get(6)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let checks = connection
                .prepare(
                    "SELECT provider_id, name, status, conclusion, url, started_at, completed_at
                     FROM check_runs WHERE pull_request_id = ?1 ORDER BY name, id",
                )?
                .query_map([&row.id], |row| {
                    Ok(CheckRun {
                        provider_id: row.get(0)?,
                        name: row.get(1)?,
                        status: row.get(2)?,
                        conclusion: row.get(3)?,
                        url: row.get(4)?,
                        started_at: row.get(5)?,
                        completed_at: row.get(6)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(Some((row, comments, checks)))
        })?;
        let Some((row, comments, checks)) = stored else {
            return Ok(None);
        };
        Ok(Some(PullRequestSnapshot {
            id: row.id,
            remote_id: row.remote_id,
            task_id: row.task_id,
            issue_id: row.issue_id,
            pull_request: PullRequest {
                provider_id: row.provider_id,
                number: u64::try_from(row.number).map_err(|_| {
                    ForgeError::InvalidResponse("stored pull request number was invalid".into())
                })?,
                title: row.title,
                body: row.body,
                state: row.state,
                url: row.url,
                head: row.head,
                base: row.base,
                draft: row.draft,
                mergeable: row.mergeable,
                head_sha: row.head_sha,
                updated_at: row.updated_at,
            },
            comments,
            checks,
        }))
    }
}

struct PullRequestRow {
    id: String,
    remote_id: String,
    task_id: Option<String>,
    issue_id: Option<String>,
    provider_id: String,
    number: i64,
    title: String,
    body: String,
    state: String,
    url: String,
    head: String,
    base: String,
    draft: bool,
    mergeable: Option<bool>,
    head_sha: String,
    updated_at: String,
}

fn now() -> Result<String, ForgeError> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| ForgeError::InvalidResponse(error.to_string()))
}

fn sql_integer(value: u64, field: &str) -> Result<i64, ForgeError> {
    i64::try_from(value).map_err(|_| {
        ForgeError::InvalidResponse(format!("{field} exceeds SQLite's signed integer range"))
    })
}
