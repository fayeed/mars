//! GitHub from the app: which remotes are on it and whether an account is
//! connected, issues and pull requests to browse, a conversation to read and
//! answer, merging, and turning an issue into a planned task.
//!
//! Provider types stay behind `mars-forge` (ADR-011); this module adds the
//! commands. Every write to the forge — a comment, a merge — is something the
//! screen states and confirms before calling, and a merge is pinned to the
//! head the person looked at.

use std::process::Command;

use mars_activity::{ActivityEntry, ActivityKind, Actor};
use mars_forge::{
    CheckRun, ForgeProvider, Issue, IssueComment, ItemState, MergeMethod, MergeOutcome,
    PullRequest, ReviewComment,
};
use mars_forge_github::GitHubProvider;
use mars_git::{PlanRequest, WorktreeProposal};
use mars_storage::projects::ProjectRepository;
use serde::Serialize;
use serde_json::json;
use tauri::State;

use crate::{AppState, CommandFailure, local_actor};

/// One remote, and what Mars can do with its forge.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteForge {
    pub name: String,
    pub url: String,
    pub host: Option<String>,
    pub owner: Option<String>,
    pub repository: Option<String>,
    /// `github` when the host is GitHub or GitHub Enterprise; `unknown` when
    /// Mars cannot read the remote's URL as a forge repository.
    pub provider: String,
    pub connected: bool,
    pub account: Option<String>,
    pub connection: Option<mars_forge::ForgeConnection>,
}

fn repository_root(
    state: &AppState,
    repository_id: &str,
) -> Result<std::path::PathBuf, CommandFailure> {
    Ok(ProjectRepository::new(state.storage.clone())
        .find_repository(repository_id)
        .map_err(|error| CommandFailure::new("repository.not_found", error.to_string()))?
        .root_path)
}

#[tauri::command]
pub async fn forge_status(
    repository_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<RemoteForge>, CommandFailure> {
    let root = repository_root(&state, &repository_id)?;
    let names = mars_git::scm::remotes(&root).unwrap_or_default();
    Ok(names
        .into_iter()
        .map(|name| {
            let url = mars_git::command::run(&root, ["remote", "get-url", &name])
                .map(|output| output.stdout_text())
                .unwrap_or_default();
            match state.forge.resolve_remote(&repository_id, &name) {
                Ok(remote) => {
                    let connection = state.forge.get_connection(&remote.id).ok().flatten();
                    RemoteForge {
                        name,
                        url,
                        host: Some(remote.repository.host.clone()),
                        owner: Some(remote.repository.owner.clone()),
                        repository: Some(remote.repository.name.clone()),
                        provider: "github".into(),
                        connected: connection.is_some(),
                        account: connection
                            .as_ref()
                            .and_then(|connection| connection.account_login.clone()),
                        connection,
                    }
                }
                Err(_) => RemoteForge {
                    name,
                    url,
                    host: None,
                    owner: None,
                    repository: None,
                    provider: "unknown".into(),
                    connected: false,
                    account: None,
                    connection: None,
                },
            }
        })
        .collect())
}

/// Connect using the GitHub CLI's own sign-in, which the person asked for by
/// clicking. The token goes straight to the OS keychain, as a pasted one
/// would; it is never written anywhere else.
#[tauri::command]
pub async fn connect_github_cli(
    repository_id: String,
    remote_name: String,
    state: State<'_, AppState>,
) -> Result<mars_forge::ForgeConnection, CommandFailure> {
    let remote = state.forge.resolve_remote(&repository_id, &remote_name)?;
    let output = mars_core::process::command("gh")
        .args(["auth", "token", "--hostname", &remote.repository.host])
        .output()
        .map_err(|_| {
            CommandFailure::new(
                "forge.gh_missing",
                "the GitHub CLI (gh) is not installed; paste a token instead",
            )
        })?;
    if !output.status.success() {
        return Err(CommandFailure::new(
            "forge.gh_signed_out",
            format!(
                "the GitHub CLI is not signed in to {}; run `gh auth login`, or paste a token",
                remote.repository.host
            ),
        ));
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    crate::connect_github_with_token(&state, &repository_id, &remote_name, &token)
}

fn provider(
    state: &AppState,
    repository_id: &str,
    remote_name: &str,
) -> Result<GitHubProvider, CommandFailure> {
    let remote = state.forge.resolve_remote(repository_id, remote_name)?;
    crate::connected_github_provider(state, &remote.id)
}

#[tauri::command]
pub async fn github_issues(
    repository_id: String,
    remote_name: String,
    item_state: ItemState,
    page: Option<u32>,
    state: State<'_, AppState>,
) -> Result<Vec<Issue>, CommandFailure> {
    Ok(provider(&state, &repository_id, &remote_name)?
        .list_issues(item_state, page.unwrap_or(1))?)
}

#[tauri::command]
pub async fn github_pull_requests(
    repository_id: String,
    remote_name: String,
    item_state: ItemState,
    page: Option<u32>,
    state: State<'_, AppState>,
) -> Result<Vec<PullRequest>, CommandFailure> {
    Ok(provider(&state, &repository_id, &remote_name)?
        .list_pull_requests(item_state, page.unwrap_or(1))?)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDetail {
    pub issue: Issue,
    pub comments: Vec<IssueComment>,
}

#[tauri::command]
pub async fn github_issue(
    repository_id: String,
    remote_name: String,
    number: u64,
    state: State<'_, AppState>,
) -> Result<IssueDetail, CommandFailure> {
    let provider = provider(&state, &repository_id, &remote_name)?;
    Ok(IssueDetail {
        issue: provider.get_issue(number)?,
        comments: provider.list_comments(number)?,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestDetail {
    pub pull: PullRequest,
    pub checks: Vec<CheckRun>,
    pub review_comments: Vec<ReviewComment>,
    pub comments: Vec<IssueComment>,
}

#[tauri::command]
pub async fn github_pull_request(
    repository_id: String,
    remote_name: String,
    number: u64,
    state: State<'_, AppState>,
) -> Result<PullRequestDetail, CommandFailure> {
    let provider = provider(&state, &repository_id, &remote_name)?;
    let pull = provider.get_pull_request(number)?;
    // Checks and line comments are secondary: a pull request is still worth
    // showing when either cannot be read.
    let checks = provider.list_checks(&pull.head_sha).unwrap_or_default();
    let review_comments = provider.list_review_comments(number).unwrap_or_default();
    let comments = provider.list_comments(number).unwrap_or_default();
    Ok(PullRequestDetail {
        pull,
        checks,
        review_comments,
        comments,
    })
}

/// Post a comment as the connected account. The screen shows who it will be
/// posted as before this is called.
#[tauri::command]
pub async fn github_comment(
    repository_id: String,
    remote_name: String,
    number: u64,
    body: String,
    state: State<'_, AppState>,
) -> Result<IssueComment, CommandFailure> {
    Ok(provider(&state, &repository_id, &remote_name)?.comment(number, &body)?)
}

/// Merge a pull request on GitHub, only if its head is still the one shown.
#[tauri::command]
pub async fn github_merge(
    repository_id: String,
    remote_name: String,
    number: u64,
    method: MergeMethod,
    expected_head: String,
    state: State<'_, AppState>,
) -> Result<MergeOutcome, CommandFailure> {
    let outcome = provider(&state, &repository_id, &remote_name)?.merge_pull_request(
        number,
        method,
        &expected_head,
    )?;
    // If a task published this pull request, its record says it landed.
    if let Ok(Some(task_id)) = task_for_pull(&state, &repository_id, number) {
        let summary = format!(
            "Merged pull request #{number} on the forge ({})",
            method.as_str()
        );
        let _ = state.activity.append(
            ActivityEntry::new(&task_id, ActivityKind::Message, Actor::Human, local_actor(), summary.clone())
                .with_payload(json!({ "text": summary, "notice": true, "forge": { "merged": number, "sha": outcome.sha } })),
        );
    }
    Ok(outcome)
}

fn task_for_pull(
    state: &AppState,
    repository_id: &str,
    number: u64,
) -> Result<Option<String>, CommandFailure> {
    use rusqlite::OptionalExtension;
    state
        .storage
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT p.task_id FROM pull_requests p JOIN remotes r ON r.id = p.remote_id
                     WHERE r.repository_id = ?1 AND p.number = ?2 AND p.task_id IS NOT NULL
                     LIMIT 1",
                    rusqlite::params![repository_id, i64::try_from(number).unwrap_or(i64::MAX)],
                    |row| row.get(0),
                )
                .optional()
        })
        .map_err(|error| CommandFailure::new("forge.read", error.to_string()))
}

/// Turn an issue into a task: imported and linked, and planned the same way
/// a request typed into the conversation is — a plan to approve before any
/// branch or worktree exists.
#[tauri::command]
pub async fn start_task_from_issue(
    repository_id: String,
    remote_name: String,
    number: u64,
    base_ref: String,
    expected_scope: Vec<String>,
    state: State<'_, AppState>,
) -> Result<WorktreeProposal, CommandFailure> {
    let provider = provider(&state, &repository_id, &remote_name)?;
    let stored = state
        .forge
        .import_issue(&repository_id, &remote_name, number, &provider)?;
    let issue = &stored.issue;
    let intent = format!(
        "{}\n\n{}\n\nFrom issue #{} ({})",
        issue.title,
        issue.body.trim(),
        issue.number,
        issue.url
    );
    let proposal = state.planner.propose(PlanRequest {
        repository_id: repository_id.clone(),
        title: format!("#{} {}", issue.number, issue.title),
        intent: intent.clone(),
        base_ref,
        expected_scope,
        setup_commands: Vec::new(),
    })?;
    state.forge.link_issue_task(&stored.id, &proposal.task_id)?;
    let _ = state.activity.append(
        ActivityEntry::new(
            &proposal.task_id,
            ActivityKind::Message,
            Actor::Human,
            local_actor(),
            format!("#{} {}", issue.number, issue.title),
        )
        .with_payload(
            json!({ "text": intent, "issue": { "number": issue.number, "url": issue.url } }),
        ),
    );
    Ok(proposal)
}

/// Open a web page in the person's browser. Only `https` links are opened,
/// and they are handed to the platform's opener as one argument, never to a
/// shell, so nothing in a link can run as a command.
#[tauri::command]
pub fn open_external_url(url: String) -> Result<(), CommandFailure> {
    let valid = url.starts_with("https://")
        && url.len() < 4096
        && !url.chars().any(|character| {
            character.is_whitespace() || character.is_control() || character == '"'
        });
    if !valid {
        return Err(CommandFailure::new(
            "open.refused",
            "only https links are opened",
        ));
    }
    let mut command = if cfg!(windows) {
        let mut command = mars_core::process::command("rundll32.exe");
        command.args(["url.dll,FileProtocolHandler", &url]);
        command
    } else if cfg!(target_os = "macos") {
        let mut command = Command::new("open");
        command.arg(&url);
        command
    } else {
        let mut command = Command::new("xdg-open");
        command.arg(&url);
        command
    };
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| CommandFailure::new("open.failed", error.to_string()))
}
