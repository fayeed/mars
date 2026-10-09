use serde::{Deserialize, Serialize};
use thiserror::Error;

mod credentials;
mod store;
mod workflow;

pub use credentials::{CredentialVault, OsCredentialVault};
pub use store::{
    ForgeConnection, ForgeMutation, ForgeStore, PullRequestSnapshot, RemoteRecord, StoredIssue,
};
pub use workflow::{ForgeWorkflow, PullRequestPlan};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgeRepository {
    pub host: String,
    pub owner: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgeUser {
    pub login: String,
    pub url: String,
}

impl ForgeRepository {
    #[must_use]
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub provider_id: String,
    pub number: u64,
    pub title: String,
    pub body: String,
    pub state: String,
    pub url: String,
    pub author: String,
    pub labels: Vec<String>,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePullRequest {
    pub title: String,
    pub body: String,
    pub head: String,
    pub base: String,
    pub draft: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub provider_id: String,
    pub number: u64,
    pub title: String,
    pub body: String,
    pub state: String,
    pub url: String,
    pub head: String,
    pub base: String,
    pub draft: bool,
    pub mergeable: Option<bool>,
    pub head_sha: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewComment {
    pub provider_id: String,
    pub author: String,
    pub body: String,
    pub path: Option<String>,
    pub line: Option<u64>,
    pub url: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckRun {
    pub provider_id: String,
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub url: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

/// A comment on an issue or on a pull request's conversation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueComment {
    pub provider_id: String,
    pub author: String,
    pub body: String,
    pub url: String,
    pub created_at: String,
}

/// Which items a list asks for.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemState {
    Open,
    Closed,
    All,
}

impl ItemState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::All => "all",
        }
    }
}

/// How a pull request is merged on the forge.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

impl MergeMethod {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Squash => "squash",
            Self::Rebase => "rebase",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeOutcome {
    pub merged: bool,
    pub sha: Option<String>,
    pub message: String,
}

#[derive(Debug, Error)]
pub enum ForgeError {
    #[error(transparent)]
    Storage(#[from] mars_storage::StorageError),
    #[error("failed to encode forge data: {0}")]
    Json(#[from] serde_json::Error),
    #[error("remote URL is not supported")]
    UnsupportedRemote(String),
    #[error("forge request failed: {0}")]
    Request(String),
    #[error("forge returned HTTP {status}: {message}")]
    Http { status: u16, message: String },
    #[error("forge response was invalid: {0}")]
    InvalidResponse(String),
    #[error("Git command failed: {0}")]
    Git(String),
    #[error("task `{0}` is not backed by a ready worktree")]
    TaskNotReady(String),
    #[error("task `{0}` has no commits to publish")]
    NoCommits(String),
    #[error("explicit approval is required before pushing and creating a pull request")]
    ApprovalRequired,
    #[error("the pull request plan changed; review a fresh preview before approving")]
    StalePlan,
    #[error("provider repository does not match the reviewed remote")]
    RepositoryMismatch,
    #[error("credential store operation failed: {0}")]
    Credential(String),
}

pub trait ForgeProvider: Send + Sync {
    fn repository(&self) -> &ForgeRepository;
    fn current_user(&self) -> Result<ForgeUser, ForgeError>;
    fn get_issue(&self, number: u64) -> Result<Issue, ForgeError>;
    fn create_pull_request(&self, input: &CreatePullRequest) -> Result<PullRequest, ForgeError>;
    fn get_pull_request(&self, number: u64) -> Result<PullRequest, ForgeError>;
    fn list_review_comments(&self, number: u64) -> Result<Vec<ReviewComment>, ForgeError>;
    fn list_checks(&self, revision: &str) -> Result<Vec<CheckRun>, ForgeError>;
    /// Ask for reviews on an open pull request.
    ///
    /// Returns the logins the forge actually requested, which is not always
    /// what was asked for: a forge silently drops a login that cannot review
    /// the repository, and the caller should report what was really done.
    fn request_reviewers(
        &self,
        number: u64,
        reviewers: &[String],
    ) -> Result<Vec<String>, ForgeError>;

    /// One page of issues, newest activity first. Pull requests are not
    /// issues here, even on forges whose API lists them together.
    fn list_issues(&self, _state: ItemState, _page: u32) -> Result<Vec<Issue>, ForgeError> {
        Err(ForgeError::Request(
            "this forge cannot list issues yet".into(),
        ))
    }

    /// One page of pull requests, newest activity first.
    fn list_pull_requests(
        &self,
        _state: ItemState,
        _page: u32,
    ) -> Result<Vec<PullRequest>, ForgeError> {
        Err(ForgeError::Request(
            "this forge cannot list pull requests yet".into(),
        ))
    }

    /// The conversation on an issue or a pull request, oldest first.
    fn list_comments(&self, _number: u64) -> Result<Vec<IssueComment>, ForgeError> {
        Err(ForgeError::Request(
            "this forge cannot list comments yet".into(),
        ))
    }

    /// Post a comment as the connected account.
    fn comment(&self, _number: u64, _body: &str) -> Result<IssueComment, ForgeError> {
        Err(ForgeError::Request(
            "this forge cannot post comments yet".into(),
        ))
    }

    /// Merge a pull request on the forge. `expected_head` guards against
    /// merging a head that moved after the person looked at it.
    fn merge_pull_request(
        &self,
        _number: u64,
        _method: MergeMethod,
        _expected_head: &str,
    ) -> Result<MergeOutcome, ForgeError> {
        Err(ForgeError::Request(
            "this forge cannot merge pull requests yet".into(),
        ))
    }
}

pub fn detect_repository(remote: &str) -> Result<ForgeRepository, ForgeError> {
    let trimmed = remote.trim().trim_end_matches('/');
    let (host, path) = if let Some(rest) = trimmed.strip_prefix("https://") {
        rest.split_once('/')
            .ok_or_else(|| ForgeError::UnsupportedRemote(remote.into()))?
    } else if let Some(rest) = trimmed.strip_prefix("http://") {
        rest.split_once('/')
            .ok_or_else(|| ForgeError::UnsupportedRemote(remote.into()))?
    } else if let Some(rest) = trimmed.strip_prefix("ssh://") {
        let (_, host_and_path) = rest.split_once('@').unwrap_or(("", rest));
        host_and_path
            .split_once('/')
            .ok_or_else(|| ForgeError::UnsupportedRemote(remote.into()))?
    } else {
        let (_, host_and_path) = trimmed
            .split_once('@')
            .ok_or_else(|| ForgeError::UnsupportedRemote(remote.into()))?;
        host_and_path
            .split_once(':')
            .ok_or_else(|| ForgeError::UnsupportedRemote(remote.into()))?
    };
    let mut segments = path.trim_matches('/').split('/');
    let owner = segments.next().unwrap_or_default();
    let raw_name = segments.next().unwrap_or_default();
    let name = raw_name.strip_suffix(".git").unwrap_or(raw_name);
    if host.is_empty()
        || host.contains('@')
        || owner.is_empty()
        || name.is_empty()
        || segments.next().is_some()
    {
        return Err(ForgeError::UnsupportedRemote(remote.into()));
    }
    Ok(ForgeRepository {
        host: host.into(),
        owner: owner.into(),
        name: name.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_https_scp_and_ssh_remotes_without_provider_leakage() {
        for (remote, host) in [
            ("https://github.com/openai/codex.git", "github.com"),
            ("git@github.com:openai/codex.git", "github.com"),
            (
                "ssh://git@git.example.test/team/mars.git",
                "git.example.test",
            ),
        ] {
            let repository = detect_repository(remote).expect("remote should detect");
            assert_eq!(repository.host, host);
            assert_eq!(
                repository.owner,
                if host == "github.com" {
                    "openai"
                } else {
                    "team"
                }
            );
        }
        assert!(detect_repository("/local/path").is_err());
        assert!(detect_repository("https://token@github.com/openai/codex.git").is_err());
        assert!(detect_repository("https://github.com/openai/codex/extra").is_err());
        assert_eq!(
            detect_repository("ssh://git@github.example.test:2222/team/mars.git")
                .expect("enterprise SSH remote should detect")
                .host,
            "github.example.test:2222"
        );
    }
}
