use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use mars_storage::Storage;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    CreatePullRequest, ForgeConnection, ForgeError, ForgeMutation, ForgeProvider, ForgeRepository,
    ForgeStore, PullRequestSnapshot, RemoteRecord, StoredIssue, detect_repository,
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestPlan {
    pub task_id: String,
    pub remote_id: String,
    pub remote_name: String,
    pub remote_url: String,
    pub push_url: String,
    pub repository: ForgeRepository,
    pub worktree_path: PathBuf,
    pub head: String,
    pub base: String,
    pub head_sha: String,
    pub commits_ahead: u64,
    pub clean: bool,
    pub title: String,
    pub body: String,
    pub draft: bool,
    pub push_command: String,
    pub fingerprint: String,
}

pub struct ForgeWorkflow {
    storage: Arc<Storage>,
    store: ForgeStore,
}

impl ForgeWorkflow {
    #[must_use]
    pub fn new(storage: Arc<Storage>) -> Self {
        Self {
            store: ForgeStore::new(storage.clone()),
            storage,
        }
    }

    pub fn import_issue(
        &self,
        repository_id: &str,
        remote_name: &str,
        number: u64,
        provider: &dyn ForgeProvider,
    ) -> Result<StoredIssue, ForgeError> {
        let remote = self.resolve_remote(repository_id, remote_name)?;
        if provider.repository() != &remote.repository {
            return Err(ForgeError::RepositoryMismatch);
        }
        let issue = provider.get_issue(number)?;
        self.store.upsert_issue(&remote.id, &issue)
    }

    pub fn link_issue_task(&self, issue_id: &str, task_id: &str) -> Result<(), ForgeError> {
        self.store.link_issue_task(issue_id, task_id)
    }

    pub fn get_issue(&self, issue_id: &str) -> Result<StoredIssue, ForgeError> {
        self.store.get_issue(issue_id)
    }

    pub fn get_task_pull_request(
        &self,
        task_id: &str,
    ) -> Result<Option<PullRequestSnapshot>, ForgeError> {
        self.store.get_task_pull_request(task_id)
    }

    pub fn get_remote(&self, remote_id: &str) -> Result<RemoteRecord, ForgeError> {
        self.store.get_remote(remote_id)
    }

    pub fn save_connection(
        &self,
        remote_id: &str,
        provider: &str,
        api_base: &str,
        account_login: Option<&str>,
        credential_ref: &str,
    ) -> Result<ForgeConnection, ForgeError> {
        self.store
            .upsert_connection(remote_id, provider, api_base, account_login, credential_ref)
    }

    pub fn get_connection(&self, remote_id: &str) -> Result<Option<ForgeConnection>, ForgeError> {
        self.store.get_connection(remote_id)
    }

    pub fn recover_interrupted(&self) -> Result<usize, ForgeError> {
        self.store.recover_interrupted_mutations()
    }

    pub fn list_task_mutations(
        &self,
        task_id: &str,
        limit: usize,
    ) -> Result<Vec<ForgeMutation>, ForgeError> {
        self.store.list_mutations(task_id, limit)
    }

    pub fn refresh_pull_request(
        &self,
        task_id: &str,
        provider: &dyn ForgeProvider,
    ) -> Result<PullRequestSnapshot, ForgeError> {
        let stored = self
            .store
            .get_task_pull_request(task_id)?
            .ok_or_else(|| ForgeError::InvalidResponse("task has no stored pull request".into()))?;
        let remote = self.store.get_remote(&stored.remote_id)?;
        if provider.repository() != &remote.repository {
            return Err(ForgeError::RepositoryMismatch);
        }
        let pull_request = provider.get_pull_request(stored.pull_request.number)?;
        let comments = provider.list_review_comments(pull_request.number)?;
        let checks = provider.list_checks(&pull_request.head_sha)?;
        self.store.record_pull_request(
            &stored.remote_id,
            stored.task_id.as_deref(),
            stored.issue_id.as_deref(),
            &pull_request,
            &comments,
            &checks,
        )
    }

    pub fn plan_pull_request(
        &self,
        task_id: &str,
        remote_name: &str,
        title: &str,
        body: &str,
        draft: bool,
    ) -> Result<PullRequestPlan, ForgeError> {
        let context = self.task_context(task_id)?;
        if context.state != "ready" {
            return Err(ForgeError::TaskNotReady(task_id.into()));
        }
        let remote = self.resolve_remote(&context.repository_id, remote_name)?;
        let push_url = git(
            &context.worktree_path,
            &["remote", "get-url", "--push", remote_name],
        )?;
        reject_authenticated_http_url(&push_url)?;
        let head_sha = git(&context.worktree_path, &["rev-parse", "HEAD"])?;
        let commits_ahead = git(
            &context.worktree_path,
            &["rev-list", "--count", &format!("{}..HEAD", context.base)],
        )?
        .parse::<u64>()
        .map_err(|error| ForgeError::Git(error.to_string()))?;
        if commits_ahead == 0 {
            return Err(ForgeError::NoCommits(task_id.into()));
        }
        let clean = git(&context.worktree_path, &["status", "--porcelain"])?.is_empty();
        let push_command = format!(
            "git push {} HEAD:refs/heads/{}",
            shell_word(&push_url),
            shell_word(&context.head)
        );
        let fingerprint = fingerprint(&[
            task_id,
            &remote.id,
            remote_name,
            &remote.fetch_url,
            &push_url,
            &context.head,
            &context.base,
            &head_sha,
            &commits_ahead.to_string(),
            &clean.to_string(),
            title,
            body,
            &draft.to_string(),
        ]);
        Ok(PullRequestPlan {
            task_id: task_id.into(),
            remote_id: remote.id,
            remote_name: remote_name.into(),
            remote_url: remote.fetch_url,
            push_url,
            repository: remote.repository,
            worktree_path: context.worktree_path,
            head: context.head,
            base: context.base,
            head_sha,
            commits_ahead,
            clean,
            title: title.into(),
            body: body.into(),
            draft,
            push_command,
            fingerprint,
        })
    }

    pub fn apply_pull_request(
        &self,
        reviewed: &PullRequestPlan,
        push_approved: bool,
        forge_approved: bool,
        provider: &dyn ForgeProvider,
    ) -> Result<PullRequestSnapshot, ForgeError> {
        if !push_approved || !forge_approved {
            return Err(ForgeError::ApprovalRequired);
        }
        let current = self.plan_pull_request(
            &reviewed.task_id,
            &reviewed.remote_name,
            &reviewed.title,
            &reviewed.body,
            reviewed.draft,
        )?;
        if current.fingerprint != reviewed.fingerprint {
            return Err(ForgeError::StalePlan);
        }
        if provider.repository() != &current.repository {
            return Err(ForgeError::RepositoryMismatch);
        }
        let started_details = serde_json::json!({
            "stage": "approved",
            "head": current.head,
            "base": current.base,
            "headSha": current.head_sha,
            "draft": current.draft,
        });
        let mutation_id = self.store.start_mutation(
            &current.task_id,
            &current.remote_id,
            &current.fingerprint,
            push_approved,
            forge_approved,
            &serde_json::to_string(&started_details)?,
        )?;
        let result = (|| {
            git(
                &current.worktree_path,
                &[
                    "push",
                    &current.push_url,
                    &format!("HEAD:refs/heads/{}", current.head),
                ],
            )?;
            let pull_request = provider.create_pull_request(&CreatePullRequest {
                title: current.title.clone(),
                body: current.body.clone(),
                head: current.head.clone(),
                base: current.base.clone(),
                draft: current.draft,
            })?;
            let issue_id = self.issue_for_task(&current.task_id)?;
            self.store.record_pull_request(
                &current.remote_id,
                Some(&current.task_id),
                issue_id.as_deref(),
                &pull_request,
                &[],
                &[],
            )?;
            let comments = provider.list_review_comments(pull_request.number)?;
            let checks = provider.list_checks(&pull_request.head_sha)?;
            self.store.record_pull_request(
                &current.remote_id,
                Some(&current.task_id),
                issue_id.as_deref(),
                &pull_request,
                &comments,
                &checks,
            )
        })();
        match result {
            Ok(snapshot) => {
                let details = serde_json::to_string(&serde_json::json!({
                    "stage": "confirmed",
                    "pullRequestNumber": snapshot.pull_request.number,
                    "pullRequestUrl": snapshot.pull_request.url,
                }))?;
                self.store
                    .finish_mutation(&mutation_id, "completed", &details, None)?;
                Ok(snapshot)
            }
            Err(error) => {
                let safe_error = mars_security::redact(&error.to_string());
                let details = serde_json::to_string(&serde_json::json!({ "stage": "failed" }))?;
                self.store
                    .finish_mutation(&mutation_id, "failed", &details, Some(&safe_error))?;
                Err(redacted_error(error))
            }
        }
    }

    pub fn resolve_remote(
        &self,
        repository_id: &str,
        remote_name: &str,
    ) -> Result<RemoteRecord, ForgeError> {
        let root: String = self.storage.with_connection(|connection| {
            connection.query_row(
                "SELECT root_path FROM repositories WHERE id = ?1",
                [repository_id],
                |row| row.get(0),
            )
        })?;
        let url = git(Path::new(&root), &["remote", "get-url", remote_name])?;
        let repository = detect_repository(&url)?;
        let provider = match repository.host.as_str() {
            "github.com" => "github",
            _ => "unknown",
        };
        self.store
            .upsert_remote(repository_id, remote_name, &url, provider, &repository)
    }

    fn task_context(&self, task_id: &str) -> Result<TaskContext, ForgeError> {
        self.storage
            .with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT t.repository_id, w.path, w.state, b.name,
                                COALESCE(NULLIF(b.base_ref, 'HEAD'), r.default_branch, b.base_ref)
                         FROM tasks t
                         JOIN repositories r ON r.id = t.repository_id
                         JOIN worktrees w ON w.task_id = t.id
                         JOIN branches b ON b.task_id = t.id
                         WHERE t.id = ?1",
                        [task_id],
                        |row| {
                            Ok(TaskContext {
                                repository_id: row.get(0)?,
                                worktree_path: PathBuf::from(row.get::<_, String>(1)?),
                                state: row.get(2)?,
                                head: row.get(3)?,
                                base: row.get(4)?,
                            })
                        },
                    )
                    .optional()
            })?
            .ok_or_else(|| ForgeError::TaskNotReady(task_id.into()))
    }

    fn issue_for_task(&self, task_id: &str) -> Result<Option<String>, ForgeError> {
        Ok(self.storage.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT issue_id FROM issue_task_links WHERE task_id = ?1",
                    [task_id],
                    |row| row.get(0),
                )
                .optional()
        })?)
    }
}

struct TaskContext {
    repository_id: String,
    worktree_path: PathBuf,
    state: String,
    head: String,
    base: String,
}

fn git(root: &Path, arguments: &[&str]) -> Result<String, ForgeError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .map_err(|error| ForgeError::Git(error.to_string()))?;
    if !output.status.success() {
        return Err(ForgeError::Git(
            String::from_utf8_lossy(&output.stderr).trim().into(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().into())
}

fn fingerprint(parts: &[&str]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update(part.as_bytes());
        digest.update([0]);
    }
    format!("{:x}", digest.finalize())
}

fn shell_word(value: &str) -> String {
    if value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"-._/".contains(&byte))
    {
        value.into()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn reject_authenticated_http_url(value: &str) -> Result<(), ForgeError> {
    let authority = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
        .and_then(|rest| rest.split('/').next());
    if authority.is_some_and(|authority| authority.contains('@')) {
        return Err(ForgeError::UnsupportedRemote(value.into()));
    }
    Ok(())
}

fn redacted_error(error: ForgeError) -> ForgeError {
    match error {
        ForgeError::UnsupportedRemote(value) => {
            ForgeError::UnsupportedRemote(mars_security::redact(&value))
        }
        ForgeError::Request(message) => ForgeError::Request(mars_security::redact(&message)),
        ForgeError::Http { status, message } => ForgeError::Http {
            status,
            message: mars_security::redact(&message),
        },
        ForgeError::InvalidResponse(message) => {
            ForgeError::InvalidResponse(mars_security::redact(&message))
        }
        ForgeError::Git(message) => ForgeError::Git(mars_security::redact(&message)),
        ForgeError::Credential(message) => ForgeError::Credential(mars_security::redact(&message)),
        other => other,
    }
}
