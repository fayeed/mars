//! Source control from the app: the main repository and every task worktree,
//! with the same operations a terminal would give, through the person's own
//! Git.
//!
//! A command names its target — the repository, or one task — and the core
//! resolves the directory from that identity rather than taking a path from
//! the webview (ADR-018). A task's worktree owns exactly one branch (ADR-005),
//! so switching or creating branches is offered on the repository only.
//! Merging a task home is previewed and fingerprinted first and is written to
//! the task's record when it lands.

use std::path::PathBuf;

use mars_activity::{ActivityEntry, ActivityKind, Actor};
use mars_git::scm::{
    self, BranchInfo, CommitInfo, IntegrationPreview, IntegrationStrategy, RepoStatus, ScmError,
    StashEntry,
};
use mars_git::{CleanupPlan, CleanupRecord};
use mars_storage::projects::ProjectRepository;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::State;

use crate::{AppState, CommandFailure, local_actor};

impl From<ScmError> for CommandFailure {
    fn from(error: ScmError) -> Self {
        let code = match error {
            ScmError::Git(_) => "git.failed",
            ScmError::Refused(_) => "git.refused",
            ScmError::PreviewChanged { .. } => "git.preview_changed",
            ScmError::Conflicts { .. } => "git.conflicts",
        };
        Self::new(code, error.to_string())
    }
}

/// Which working directory a command acts on.
#[derive(Clone, Debug, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum GitTarget {
    Repository { repository_id: String },
    Task { task_id: String },
}

impl GitTarget {
    fn task_id(&self) -> Option<&str> {
        match self {
            Self::Task { task_id } => Some(task_id),
            Self::Repository { .. } => None,
        }
    }
}

fn root_of(state: &AppState, target: &GitTarget) -> Result<PathBuf, CommandFailure> {
    match target {
        GitTarget::Repository { repository_id } => {
            Ok(ProjectRepository::new(state.storage.clone())
                .find_repository(repository_id)
                .map_err(|error| CommandFailure::new("repository.not_found", error.to_string()))?
                .root_path)
        }
        GitTarget::Task { task_id } => {
            let task = state.tasks.get(task_id)?;
            if task.state != mars_git::WorktreeState::Ready || !task.worktree_path.is_dir() {
                return Err(CommandFailure::new(
                    "git.worktree_missing",
                    "this task's worktree does not exist right now",
                ));
            }
            Ok(task.worktree_path)
        }
    }
}

/// Write what was done to a task's record, so the conversation shows it.
fn note(state: &AppState, target: &GitTarget, summary: String, payload: serde_json::Value) {
    if let Some(task_id) = target.task_id() {
        let _ = state.activity.append(
            ActivityEntry::new(
                task_id,
                ActivityKind::Message,
                Actor::Human,
                local_actor(),
                summary.clone(),
            )
            .with_payload(json!({ "text": summary, "git": payload, "notice": true })),
        );
    }
}

#[tauri::command]
pub async fn git_status(
    target: GitTarget,
    state: State<'_, AppState>,
) -> Result<RepoStatus, CommandFailure> {
    Ok(scm::status(&root_of(&state, &target)?)?)
}

#[tauri::command]
pub async fn git_diff(
    target: GitTarget,
    path: Option<String>,
    staged: bool,
    state: State<'_, AppState>,
) -> Result<String, CommandFailure> {
    Ok(scm::diff(
        &root_of(&state, &target)?,
        path.as_deref(),
        staged,
    )?)
}

#[tauri::command]
pub async fn git_stage(
    target: GitTarget,
    paths: Vec<String>,
    state: State<'_, AppState>,
) -> Result<RepoStatus, CommandFailure> {
    let root = root_of(&state, &target)?;
    scm::stage(&root, &paths)?;
    Ok(scm::status(&root)?)
}

#[tauri::command]
pub async fn git_unstage(
    target: GitTarget,
    paths: Vec<String>,
    state: State<'_, AppState>,
) -> Result<RepoStatus, CommandFailure> {
    let root = root_of(&state, &target)?;
    scm::unstage(&root, &paths)?;
    Ok(scm::status(&root)?)
}

/// Throw away working-tree changes. The screen confirms first: this is the
/// one operation here that cannot be undone.
#[tauri::command]
pub async fn git_discard(
    target: GitTarget,
    paths: Vec<String>,
    state: State<'_, AppState>,
) -> Result<RepoStatus, CommandFailure> {
    let root = root_of(&state, &target)?;
    scm::discard(&root, &paths)?;
    note(
        &state,
        &target,
        format!("Discarded changes to {}", paths.join(", ")),
        json!({ "operation": "discard", "paths": paths }),
    );
    Ok(scm::status(&root)?)
}

#[tauri::command]
pub async fn git_commit(
    target: GitTarget,
    message: String,
    amend: bool,
    state: State<'_, AppState>,
) -> Result<CommitInfo, CommandFailure> {
    let root = root_of(&state, &target)?;
    let commit = scm::commit(&root, &message, amend)?;
    note(
        &state,
        &target,
        format!("Committed {} — {}", commit.short_oid, commit.subject),
        json!({ "operation": "commit", "oid": commit.oid, "amend": amend }),
    );
    Ok(commit)
}

#[tauri::command]
pub async fn git_log(
    target: GitTarget,
    limit: Option<usize>,
    skip: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<CommitInfo>, CommandFailure> {
    Ok(scm::log(
        &root_of(&state, &target)?,
        None,
        limit.unwrap_or(50),
        skip.unwrap_or(0),
    )?)
}

#[tauri::command]
pub async fn git_show(
    target: GitTarget,
    oid: String,
    state: State<'_, AppState>,
) -> Result<String, CommandFailure> {
    Ok(scm::show(&root_of(&state, &target)?, &oid)?)
}

#[tauri::command]
pub async fn git_branches(
    target: GitTarget,
    state: State<'_, AppState>,
) -> Result<Vec<BranchInfo>, CommandFailure> {
    Ok(scm::branches(&root_of(&state, &target)?)?)
}

fn repository_only(target: &GitTarget) -> Result<(), CommandFailure> {
    if target.task_id().is_some() {
        return Err(CommandFailure::new(
            "git.task_branch",
            "a task's worktree stays on its own branch; switch branches in the repository instead",
        ));
    }
    Ok(())
}

#[tauri::command]
pub async fn git_switch(
    target: GitTarget,
    branch: String,
    state: State<'_, AppState>,
) -> Result<RepoStatus, CommandFailure> {
    repository_only(&target)?;
    let root = root_of(&state, &target)?;
    scm::switch(&root, &branch)?;
    Ok(scm::status(&root)?)
}

#[tauri::command]
pub async fn git_create_branch(
    target: GitTarget,
    name: String,
    start: Option<String>,
    state: State<'_, AppState>,
) -> Result<RepoStatus, CommandFailure> {
    repository_only(&target)?;
    let root = root_of(&state, &target)?;
    scm::create_branch(&root, &name, start.as_deref())?;
    Ok(scm::status(&root)?)
}

#[tauri::command]
pub async fn git_remotes(
    target: GitTarget,
    state: State<'_, AppState>,
) -> Result<Vec<String>, CommandFailure> {
    Ok(scm::remotes(&root_of(&state, &target)?)?)
}

#[tauri::command]
pub async fn git_fetch(
    target: GitTarget,
    state: State<'_, AppState>,
) -> Result<RepoStatus, CommandFailure> {
    let root = root_of(&state, &target)?;
    scm::fetch(&root)?;
    Ok(scm::status(&root)?)
}

#[tauri::command]
pub async fn git_pull(
    target: GitTarget,
    state: State<'_, AppState>,
) -> Result<RepoStatus, CommandFailure> {
    let root = root_of(&state, &target)?;
    scm::pull(&root)?;
    note(
        &state,
        &target,
        "Pulled from upstream".into(),
        json!({ "operation": "pull" }),
    );
    Ok(scm::status(&root)?)
}

/// Push one branch to one remote. The screen states both and asks before it
/// calls this (ADR-012).
#[tauri::command]
pub async fn git_push(
    target: GitTarget,
    remote: String,
    branch: String,
    state: State<'_, AppState>,
) -> Result<RepoStatus, CommandFailure> {
    let root = root_of(&state, &target)?;
    scm::push(&root, &remote, &branch)?;
    note(
        &state,
        &target,
        format!("Pushed {branch} to {remote}"),
        json!({ "operation": "push", "remote": remote, "branch": branch }),
    );
    Ok(scm::status(&root)?)
}

#[tauri::command]
pub async fn git_stashes(
    target: GitTarget,
    state: State<'_, AppState>,
) -> Result<Vec<StashEntry>, CommandFailure> {
    Ok(scm::stashes(&root_of(&state, &target)?)?)
}

#[tauri::command]
pub async fn git_stash_push(
    target: GitTarget,
    message: String,
    state: State<'_, AppState>,
) -> Result<RepoStatus, CommandFailure> {
    let root = root_of(&state, &target)?;
    scm::stash_push(&root, &message)?;
    Ok(scm::status(&root)?)
}

#[tauri::command]
pub async fn git_stash_pop(
    target: GitTarget,
    state: State<'_, AppState>,
) -> Result<RepoStatus, CommandFailure> {
    let root = root_of(&state, &target)?;
    scm::stash_pop(&root)?;
    Ok(scm::status(&root)?)
}

// --- bringing a task home ------------------------------------------------

/// The base a task merges into, as a branch name. A task planned from a
/// detached HEAD or a commit id has no branch to merge into, and says so.
fn task_integration(
    state: &AppState,
    task_id: &str,
) -> Result<(PathBuf, String, String), CommandFailure> {
    let task = state.tasks.get(task_id)?;
    let root = ProjectRepository::new(state.storage.clone())
        .find_repository(&task.repository_id)
        .map_err(|error| CommandFailure::new("repository.not_found", error.to_string()))?
        .root_path;
    let base = task.base_ref.clone();
    let is_branch = mars_git::command::run(
        &root,
        [
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{base}"),
        ],
    )
    .is_ok();
    if !is_branch {
        return Err(CommandFailure::new(
            "git.base_not_branch",
            format!(
                "this task was based on `{base}`, which is not a branch, so there is no branch to merge it into; push it and open a pull request instead"
            ),
        ));
    }
    Ok((root, base, task.branch_name))
}

#[tauri::command]
pub async fn integration_preview(
    task_id: String,
    state: State<'_, AppState>,
) -> Result<IntegrationPreview, CommandFailure> {
    let (root, base, head) = task_integration(&state, &task_id)?;
    Ok(scm::preview_integration(&root, &base, &head)?)
}

/// Merge a task's branch into its base as previewed, and record it on the
/// task. On conflict nothing changes and the conflicting files are named.
#[tauri::command]
pub async fn integration_apply(
    task_id: String,
    fingerprint: String,
    strategy: IntegrationStrategy,
    message: String,
    state: State<'_, AppState>,
) -> Result<CommitInfo, CommandFailure> {
    let (root, base, head) = task_integration(&state, &task_id)?;
    let merged = scm::apply_integration(&root, &base, &head, &fingerprint, strategy, &message)?;
    let summary = format!(
        "Merged {head} into {base} as {} ({})",
        merged.short_oid,
        match strategy {
            IntegrationStrategy::Merge => "merge commit",
            IntegrationStrategy::Squash => "squashed",
            IntegrationStrategy::FastForward => "fast-forward",
        }
    );
    let _ = state.activity.append(
        ActivityEntry::new(
            &task_id,
            ActivityKind::Message,
            Actor::Human,
            local_actor(),
            summary.clone(),
        )
        .with_payload(json!({
            "text": summary,
            "notice": true,
            "git": { "operation": "integrate", "base": base, "head": head, "oid": merged.oid },
        })),
    );
    Ok(merged)
}

// --- removing and restoring a worktree ------------------------------------

#[tauri::command]
pub async fn worktree_cleanup_plan(
    task_id: String,
    state: State<'_, AppState>,
) -> Result<CleanupPlan, CommandFailure> {
    Ok(state.planner.plan_cleanup(&task_id)?)
}

/// Remove a task's worktree as previewed. The branch tip is archived first,
/// so it can be restored (ADR-041).
#[tauri::command]
pub async fn worktree_cleanup(
    task_id: String,
    fingerprint: String,
    discard_uncommitted: bool,
    state: State<'_, AppState>,
) -> Result<CleanupRecord, CommandFailure> {
    Ok(state
        .planner
        .cleanup(&task_id, &fingerprint, discard_uncommitted, &local_actor())?)
}

#[tauri::command]
pub async fn worktree_restore(
    cleanup_id: String,
    state: State<'_, AppState>,
) -> Result<CleanupRecord, CommandFailure> {
    Ok(state.planner.restore(&cleanup_id, &local_actor())?)
}

/// The most recent removal of a task's worktree, which can be restored.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupView {
    pub id: String,
    pub removed_path: String,
    pub branch_name: String,
    pub archived_ref: Option<String>,
    pub state: String,
}

#[tauri::command]
pub async fn worktree_last_cleanup(
    task_id: String,
    state: State<'_, AppState>,
) -> Result<Option<CleanupView>, CommandFailure> {
    state
        .storage
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT id, removed_path, branch_name, archived_ref, state
                     FROM worktree_cleanups WHERE task_id = ?1
                     ORDER BY rowid DESC LIMIT 1",
                    [&task_id],
                    |row| {
                        Ok(CleanupView {
                            id: row.get(0)?,
                            removed_path: row.get(1)?,
                            branch_name: row.get(2)?,
                            archived_ref: row.get(3)?,
                            state: row.get(4)?,
                        })
                    },
                )
                .optional()
        })
        .map_err(|error| CommandFailure::new("cleanup.read", error.to_string()))
}

/// Everything a task changed since it left its base, committed or not.
pub fn task_change(
    state: &AppState,
    task: &mars_git::WorktreePlan,
) -> Result<(String, Vec<String>), CommandFailure> {
    // The commit the plan pinned is the truthful base; a task made before
    // plans were pinned falls back to its base ref.
    let base = state
        .planner
        .current(&task.task_id)
        .map(|plan| plan.base_oid)
        .ok()
        .filter(|oid| !oid.is_empty())
        .unwrap_or_else(|| task.base_ref.clone());
    match scm::change_since(&task.worktree_path, &base) {
        Ok(change) => Ok(change),
        // A base the worktree cannot see falls back to HEAD rather than
        // failing the review outright; the record of committed work is then
        // the commit list, which the publish preview shows.
        Err(_) => scm::change_since(&task.worktree_path, "HEAD").map_err(Into::into),
    }
}
