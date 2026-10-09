//! Worktree plans: what a human approves before Mars touches Git.
//!
//! A [`WorktreeProposal`] is produced without mutating anything. It names the
//! base ref *and the commit that ref currently points at*, the branch, the path,
//! the file scope the task expects to stay inside, and the setup commands that
//! will run once the checkout exists. It also carries preflight findings, each
//! with the resolutions available and what each one would cost.
//!
//! The plan is fingerprinted. Approval is recorded against that fingerprint, and
//! provisioning rebuilds the plan and refuses to act if it no longer matches —
//! so an approval can never be replayed against a plan that changed underneath
//! it.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mars_activity::{ActivityEntry, ActivityKind, ActivityRecord, Actor, Attachment};
use mars_security::{CommandClass, classify_command};
use mars_storage::Storage;
use mars_storage::projects::{ProjectRecord, ProjectRepository};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::command::{GitCommandError, run};
use crate::disk::{DiskReport, report_for};
use crate::status::status_is_dirty;
use crate::worktree::{TaskWorktreeError, TaskWorktreeService, WorktreePlan, WorktreeState};

/// One command the plan will run in the new worktree once it exists.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupCommand {
    pub command: String,
    pub purpose: String,
    /// Filled in by the planner so the human sees what they are approving.
    pub classification: String,
}

impl SetupCommand {
    #[must_use]
    pub fn new(command: impl Into<String>, purpose: impl Into<String>) -> Self {
        let command = command.into();
        let classification = classify_command(&command).class.as_str().to_owned();
        Self {
            command,
            purpose: purpose.into(),
            classification,
        }
    }
}

/// What the planner found that a person should know about before approving.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreflightCode {
    /// The base ref moved after this plan was built.
    BaseMoved,
    /// The repository the worktree branches from has uncommitted changes.
    DirtyBase,
    /// A worktree is already registered at, or a directory already occupies,
    /// the planned path.
    WorktreeExists,
    /// A branch with the planned name already exists in Git.
    BranchExists,
    /// The volume is close to full.
    LowDisk,
    /// A setup command would do more than install dependencies.
    RiskySetupCommand,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreflightSeverity {
    /// Worth seeing; provisioning will still work.
    Warning,
    /// Provisioning cannot proceed until it is resolved.
    Blocking,
}

/// One way out of a preflight finding, and what taking it costs.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Resolution {
    pub action: String,
    pub consequence: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightFinding {
    pub code: PreflightCode,
    pub severity: PreflightSeverity,
    pub detail: String,
    pub resolutions: Vec<Resolution>,
}

/// What a person asks for.
#[derive(Clone, Debug)]
pub struct PlanRequest {
    pub repository_id: String,
    pub title: String,
    pub intent: String,
    pub base_ref: String,
    /// Globs the task expects to stay inside. A write outside these goes to the
    /// permission broker rather than happening quietly.
    pub expected_scope: Vec<String>,
    pub setup_commands: Vec<SetupCommand>,
}

/// The mutation-free plan, exactly as the approval dialog shows it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeProposal {
    pub task_id: String,
    pub repository_id: String,
    pub title: String,
    pub intent: String,
    pub base_ref: String,
    /// The commit `base_ref` resolved to when this plan was built.
    pub base_oid: String,
    pub branch_name: String,
    pub worktree_path: PathBuf,
    pub expected_scope: Vec<String>,
    pub setup_commands: Vec<SetupCommand>,
    pub preflight: Vec<PreflightFinding>,
    pub disk: DiskReport,
    pub state: WorktreeState,
    /// Digest over every field above. Approval is recorded against this.
    pub fingerprint: String,
    pub approved_by: Option<String>,
    pub approved_at: Option<String>,
}

impl WorktreeProposal {
    /// Whether anything blocks provisioning.
    #[must_use]
    pub fn is_blocked(&self) -> bool {
        self.preflight
            .iter()
            .any(|finding| finding.severity == PreflightSeverity::Blocking)
    }

    #[must_use]
    pub fn is_approved(&self) -> bool {
        self.approved_by.is_some()
    }
}

/// The base ref moved. Each choice says what it does to the work already done.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftChoice {
    /// Replay this branch's commits onto the new base.
    Rebase,
    /// Throw the branch away and cut a new one from the new base.
    Recreate,
    /// Keep working from the old base and diverge.
    ContinueAndDiverge,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseDrift {
    pub base_ref: String,
    pub planned_oid: String,
    pub current_oid: String,
    pub commits_behind: usize,
    pub options: Vec<DriftOption>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriftOption {
    pub choice: DriftChoice,
    pub label: String,
    pub consequence: String,
}

/// What removing a worktree would do, before it does it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupPlan {
    pub task_id: String,
    pub worktree_path: PathBuf,
    pub branch_name: String,
    pub branch_oid: Option<String>,
    /// Where the branch tip will be archived so this can be undone.
    pub archive_ref: String,
    /// Files with uncommitted changes that removal would discard.
    pub uncommitted: Vec<String>,
    pub commits_ahead_of_base: usize,
    /// True when nothing would be lost that the archive cannot bring back.
    pub reversible: bool,
    pub fingerprint: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupRecord {
    pub id: String,
    pub task_id: String,
    pub removed_path: PathBuf,
    pub branch_name: String,
    pub archived_ref: Option<String>,
    pub archived_oid: Option<String>,
    pub state: String,
}

/// Builds, approves, provisions, and cleans up worktree plans.
pub struct WorktreePlanner {
    storage: Arc<Storage>,
    projects: ProjectRepository,
    worktrees: TaskWorktreeService,
    activity: Option<ActivityRecord>,
}

impl WorktreePlanner {
    #[must_use]
    pub fn new(storage: Arc<Storage>, workspace_root: impl AsRef<Path>) -> Self {
        Self {
            projects: ProjectRepository::new(storage.clone()),
            worktrees: TaskWorktreeService::new(storage.clone(), workspace_root),
            storage,
            activity: None,
        }
    }

    #[must_use]
    pub fn recording_to(mut self, activity: ActivityRecord) -> Self {
        self.activity = Some(activity);
        self
    }

    #[must_use]
    pub fn worktrees(&self) -> &TaskWorktreeService {
        &self.worktrees
    }

    /// Build a plan. Writes plan rows; touches Git only to read.
    pub fn propose(&self, request: PlanRequest) -> Result<WorktreeProposal, TaskWorktreeError> {
        let plan = self.worktrees.create_task(
            &request.repository_id,
            &request.title,
            &request.intent,
            &request.base_ref,
        )?;
        let repository = self.projects.find_repository(&request.repository_id)?;
        let base_oid = self.resolve_base(&repository, &request.base_ref, &request.repository_id)?;

        let proposal = self.assemble(&plan, &repository, base_oid, &request)?;
        self.persist(&proposal)?;
        self.record(PlanEvent {
            task_id: &proposal.task_id,
            kind: ActivityKind::WorktreePlanned,
            actor: Actor::System,
            actor_label: "mars",
            summary: &format!(
                "Planned {} from {} ({})",
                proposal.branch_name,
                proposal.base_ref,
                short(&proposal.base_oid)
            ),
            payload: json!({
                "branchName": proposal.branch_name,
                "baseRef": proposal.base_ref,
                "baseOid": proposal.base_oid,
                "path": proposal.worktree_path,
                "fingerprint": proposal.fingerprint,
                "expectedScope": proposal.expected_scope,
                "setupCommands": proposal.setup_commands,
                "preflight": proposal.preflight,
            }),
            attachment: None,
        })?;
        Ok(proposal)
    }

    /// Rebuild the plan from durable state, refreshing everything that can move
    /// underneath it: the base commit, the path, the branch, the disk.
    pub fn current(&self, task_id: &str) -> Result<WorktreeProposal, TaskWorktreeError> {
        let stored = self.stored(task_id)?;
        let plan = self.worktrees.get(task_id)?;
        let repository = self.projects.find_repository(&plan.repository_id)?;
        // The plan keeps the commit it pinned. What the ref points at *now* is a
        // separate fact, and a difference between the two is a finding rather
        // than a silent redefinition of what was approved.
        let live_oid = self
            .resolve_base(&repository, &plan.base_ref, &plan.repository_id)
            .unwrap_or_else(|_| stored.base_oid.clone());

        let request = PlanRequest {
            repository_id: plan.repository_id.clone(),
            title: plan.title.clone(),
            intent: plan.intent.clone(),
            base_ref: plan.base_ref.clone(),
            expected_scope: stored.expected_scope.clone(),
            setup_commands: stored.setup_commands.clone(),
        };
        let mut proposal = self.assemble(&plan, &repository, stored.base_oid.clone(), &request)?;

        if live_oid != stored.base_oid
            && stored.acknowledged_drift_oid.as_deref() != Some(live_oid.as_str())
        {
            proposal.preflight.push(PreflightFinding {
                code: PreflightCode::BaseMoved,
                severity: PreflightSeverity::Blocking,
                detail: format!(
                    "{} pointed at {} when this plan was built and now points at {}",
                    proposal.base_ref,
                    short(&stored.base_oid),
                    short(&live_oid)
                ),
                resolutions: drift_options(proposal.state)
                    .into_iter()
                    .map(|option| Resolution {
                        action: option.label,
                        consequence: option.consequence,
                    })
                    .collect(),
            });
        }
        proposal.approved_by = stored.approved_by;
        proposal.approved_at = stored.approved_at;
        Ok(proposal)
    }

    /// Change what a plan declares, before its worktree exists.
    ///
    /// Approval is recorded against a fingerprint, and both of these fields are
    /// inside it, so amending drops any approval the plan had: the plan a person
    /// said yes to is not the plan any more, and they are asked again.
    pub fn amend(
        &self,
        task_id: &str,
        expected_scope: Vec<String>,
        setup_commands: Vec<SetupCommand>,
    ) -> Result<WorktreeProposal, TaskWorktreeError> {
        let plan = self.worktrees.get(task_id)?;
        if plan.state != WorktreeState::Planned {
            return Err(TaskWorktreeError::InvalidTransition {
                task_id: task_id.to_owned(),
                state: plan.state.as_str().to_owned(),
                operation: "amend a plan whose worktree already exists",
            });
        }
        let stored = self.stored(task_id)?;
        let repository = self.projects.find_repository(&plan.repository_id)?;
        let request = PlanRequest {
            repository_id: plan.repository_id.clone(),
            title: plan.title.clone(),
            intent: plan.intent.clone(),
            base_ref: plan.base_ref.clone(),
            expected_scope,
            setup_commands,
        };
        // The base commit the plan pinned is kept. Amending changes what the
        // task declares, not what it branches from.
        let proposal = self.assemble(&plan, &repository, stored.base_oid, &request)?;
        self.persist(&proposal)?;
        self.storage.with_connection(|connection| {
            connection.execute(
                "UPDATE worktree_plans
                 SET approved_by = NULL, approved_at = NULL, approved_fingerprint = NULL,
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE task_id = ?1",
                params![task_id],
            )?;
            Ok(())
        })?;
        self.current(task_id)
    }

    /// Record a human's approval of the exact plan they were shown.
    pub fn approve(
        &self,
        task_id: &str,
        fingerprint: &str,
        approved_by: &str,
    ) -> Result<WorktreeProposal, TaskWorktreeError> {
        let proposal = self.current(task_id)?;
        if proposal.fingerprint != fingerprint {
            return Err(TaskWorktreeError::PlanChanged {
                task_id: task_id.to_owned(),
                approved: fingerprint.to_owned(),
                current: proposal.fingerprint,
            });
        }
        if proposal.is_blocked() {
            return Err(TaskWorktreeError::PlanBlocked {
                task_id: task_id.to_owned(),
                detail: blocking_detail(&proposal),
            });
        }
        self.storage.with_connection(|connection| {
            connection.execute(
                "UPDATE worktree_plans
                 SET approved_by = ?2, approved_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                     approved_fingerprint = ?3,
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE task_id = ?1",
                params![task_id, approved_by, fingerprint],
            )?;
            Ok(())
        })?;
        self.current(task_id)
    }

    /// Provision the approved plan, then run its setup commands in the new
    /// worktree. Refuses if the plan has changed since it was approved.
    pub fn provision_approved(&self, task_id: &str) -> Result<WorktreeProposal, TaskWorktreeError> {
        let stored = self.stored(task_id)?;
        let approved_fingerprint =
            stored
                .approved_fingerprint
                .ok_or_else(|| TaskWorktreeError::PlanNotApproved {
                    task_id: task_id.to_owned(),
                })?;
        let proposal = self.current(task_id)?;
        if proposal.fingerprint != approved_fingerprint {
            return Err(TaskWorktreeError::PlanChanged {
                task_id: task_id.to_owned(),
                approved: approved_fingerprint,
                current: proposal.fingerprint,
            });
        }

        if proposal.is_blocked() {
            return Err(TaskWorktreeError::PlanBlocked {
                task_id: task_id.to_owned(),
                detail: blocking_detail(&proposal),
            });
        }

        let provisioned = self.worktrees.provision(task_id)?;
        self.record(PlanEvent {
            task_id,
            kind: ActivityKind::WorktreeCreated,
            actor: Actor::System,
            actor_label: "mars",
            summary: &format!(
                "Created {} at {}",
                provisioned.branch_name,
                provisioned.worktree_path.display()
            ),
            payload: json!({
                "branchName": provisioned.branch_name,
                "baseRef": provisioned.base_ref,
                "baseOid": proposal.base_oid,
                "path": provisioned.worktree_path,
                "approvedBy": stored.approved_by,
            }),
            attachment: None,
        })?;

        self.run_setup_commands(task_id, &provisioned, &proposal.setup_commands)?;
        self.current(task_id)
    }

    /// Whether the base has moved out from under a task, and what can be done.
    pub fn base_drift(&self, task_id: &str) -> Result<Option<BaseDrift>, TaskWorktreeError> {
        let stored = self.stored(task_id)?;
        let plan = self.worktrees.get(task_id)?;
        let repository = self.projects.find_repository(&plan.repository_id)?;
        let current_oid = self.resolve_base(&repository, &plan.base_ref, &plan.repository_id)?;
        if current_oid == stored.base_oid {
            return Ok(None);
        }
        let range = format!("{}..{current_oid}", stored.base_oid);
        let commits_behind = run(&repository.root_path, ["rev-list", "--count", &range])
            .map(|output| output.stdout_text().parse::<usize>().unwrap_or(0))
            .unwrap_or(0);
        Ok(Some(BaseDrift {
            base_ref: plan.base_ref,
            planned_oid: stored.base_oid,
            current_oid,
            commits_behind,
            options: drift_options(plan.state),
        }))
    }

    /// Act on drift. Every branch of this records what was chosen and why.
    pub fn resolve_base_drift(
        &self,
        task_id: &str,
        choice: DriftChoice,
        decided_by: &str,
    ) -> Result<WorktreeProposal, TaskWorktreeError> {
        let Some(drift) = self.base_drift(task_id)? else {
            return self.current(task_id);
        };
        let plan = self.worktrees.get(task_id)?;
        let repository = self.projects.find_repository(&plan.repository_id)?;

        match choice {
            DriftChoice::Recreate if plan.state == WorktreeState::Planned => {
                // Nothing exists yet, so moving the plan onto the new base is
                // the whole of the work. It needs approving again.
                self.rebase_plan(task_id, &drift.current_oid)?;
            }
            DriftChoice::Recreate => {
                return Err(TaskWorktreeError::InvalidTransition {
                    task_id: task_id.to_owned(),
                    state: plan.state.as_str().to_owned(),
                    operation: "recreate a provisioned worktree",
                });
            }
            DriftChoice::Rebase => {
                if plan.state != WorktreeState::Ready {
                    return Err(TaskWorktreeError::InvalidTransition {
                        task_id: task_id.to_owned(),
                        state: plan.state.as_str().to_owned(),
                        operation: "rebase a worktree that does not exist",
                    });
                }
                run(
                    &plan.worktree_path,
                    [
                        OsString::from("rebase"),
                        OsString::from("--"),
                        OsString::from(&drift.current_oid),
                    ],
                )?;
                self.rebase_plan(task_id, &drift.current_oid)?;
            }
            DriftChoice::ContinueAndDiverge => {
                self.storage.with_connection(|connection| {
                    connection.execute(
                        "UPDATE worktree_plans
                         SET acknowledged_drift_oid = ?2,
                             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                         WHERE task_id = ?1",
                        params![task_id, &drift.current_oid],
                    )?;
                    Ok(())
                })?;
            }
        }

        let _ = repository;
        self.record(PlanEvent {
            task_id,
            kind: ActivityKind::WorktreeCreated,
            actor: Actor::Human,
            actor_label: decided_by,
            summary: &format!(
                "Base {} moved from {} to {}; chose {}",
                drift.base_ref,
                short(&drift.planned_oid),
                short(&drift.current_oid),
                match choice {
                    DriftChoice::Rebase => "rebase",
                    DriftChoice::Recreate => "recreate",
                    DriftChoice::ContinueAndDiverge => "continue and diverge",
                }
            ),
            payload: json!({
                "branchName": plan.branch_name,
                "baseRef": drift.base_ref,
                "baseOid": match choice {
                    DriftChoice::ContinueAndDiverge => drift.planned_oid.clone(),
                    _ => drift.current_oid.clone(),
                },
                "path": plan.worktree_path,
                "driftChoice": choice,
                "decidedBy": decided_by,
            }),
            attachment: None,
        })?;
        self.current(task_id)
    }

    /// What removing this worktree would cost, computed without removing it.
    pub fn plan_cleanup(&self, task_id: &str) -> Result<CleanupPlan, TaskWorktreeError> {
        let plan = self.worktrees.get(task_id)?;
        let stored = self.stored(task_id)?;
        let repository = self.projects.find_repository(&plan.repository_id)?;

        let uncommitted = if plan.worktree_path.is_dir() {
            let status = run(
                &plan.worktree_path,
                ["status", "--porcelain=v1", "-z", "--untracked-files=all"],
            )?;
            parse_status_paths(&status.stdout)
        } else {
            Vec::new()
        };
        let branch_oid = run(
            &repository.root_path,
            [
                "rev-parse",
                "--verify",
                "--end-of-options",
                &plan.branch_name,
            ],
        )
        .ok()
        .map(|output| output.stdout_text());
        let commits_ahead = branch_oid
            .as_ref()
            .and_then(|_| {
                let range = format!("{}..{}", stored.base_oid, plan.branch_name);
                run(&repository.root_path, ["rev-list", "--count", &range]).ok()
            })
            .map(|output| output.stdout_text().parse::<usize>().unwrap_or(0))
            .unwrap_or(0);

        let archive_ref = format!("refs/mars/archive/{}", plan.branch_name);
        let mut cleanup = CleanupPlan {
            task_id: task_id.to_owned(),
            worktree_path: plan.worktree_path,
            branch_name: plan.branch_name,
            branch_oid,
            archive_ref,
            uncommitted,
            commits_ahead_of_base: commits_ahead,
            // Committed work survives in the archive ref; uncommitted work does
            // not, and that is the whole of the irreversibility.
            reversible: true,
            fingerprint: String::new(),
        };
        cleanup.reversible = cleanup.uncommitted.is_empty();
        cleanup.fingerprint = fingerprint_of(&cleanup);
        Ok(cleanup)
    }

    /// Remove the worktree after archiving its branch tip. Refuses to discard
    /// uncommitted work unless the caller says so explicitly.
    pub fn cleanup(
        &self,
        task_id: &str,
        fingerprint: &str,
        discard_uncommitted: bool,
        decided_by: &str,
    ) -> Result<CleanupRecord, TaskWorktreeError> {
        let cleanup = self.plan_cleanup(task_id)?;
        if cleanup.fingerprint != fingerprint {
            return Err(TaskWorktreeError::PlanChanged {
                task_id: task_id.to_owned(),
                approved: fingerprint.to_owned(),
                current: cleanup.fingerprint,
            });
        }
        if !cleanup.uncommitted.is_empty() && !discard_uncommitted {
            return Err(TaskWorktreeError::DirtyWorktree {
                path: cleanup.worktree_path,
            });
        }

        let plan = self.worktrees.get(task_id)?;
        let repository = self.projects.find_repository(&plan.repository_id)?;

        // Archive first. If anything after this fails, the branch tip is already
        // recoverable.
        if let Some(oid) = cleanup.branch_oid.as_deref() {
            run(
                &repository.root_path,
                [
                    OsString::from("update-ref"),
                    OsString::from(&cleanup.archive_ref),
                    OsString::from(oid),
                ],
            )?;
        }

        let id = Uuid::now_v7().to_string();
        let preserved = serde_json::to_string(&cleanup)?;
        self.storage.with_connection(|connection| {
            connection.execute(
                "INSERT INTO worktree_cleanups
                     (id, task_id, removed_path, branch_name, archived_ref, archived_oid,
                      preserved_json, state, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'planned',
                         strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                params![
                    &id,
                    task_id,
                    cleanup.worktree_path.to_string_lossy(),
                    &cleanup.branch_name,
                    cleanup.branch_oid.as_ref().map(|_| &cleanup.archive_ref),
                    cleanup.branch_oid.as_deref(),
                    &preserved,
                ],
            )?;
            Ok(())
        })?;

        // Removal happens here rather than through `TaskWorktreeService::remove`,
        // which refuses to delete a branch holding commits its base does not.
        // That refusal is right when the commits would be lost; here they are
        // already archived above, so the branch can go.
        let mut arguments = vec![OsString::from("worktree"), OsString::from("remove")];
        if discard_uncommitted && !cleanup.uncommitted.is_empty() {
            arguments.push(OsString::from("--force"));
        }
        arguments.push(cleanup.worktree_path.clone().into_os_string());

        if let Err(error) = run(&repository.root_path, arguments) {
            self.finish_cleanup(&id, "failed")?;
            return Err(error.into());
        }
        if cleanup.branch_oid.is_some()
            && let Err(error) = run(
                &repository.root_path,
                ["branch", "-D", &cleanup.branch_name],
            )
        {
            self.finish_cleanup(&id, "failed")?;
            return Err(error.into());
        }
        self.mark_removed(task_id)?;
        self.finish_cleanup(&id, "completed")?;

        self.record(PlanEvent {
            task_id,
            kind: ActivityKind::WorktreeRemoved,
            actor: Actor::Human,
            actor_label: decided_by,
            summary: &format!(
                "Removed {} ({} archived at {})",
                cleanup.worktree_path.display(),
                cleanup.branch_name,
                cleanup.archive_ref
            ),
            payload: json!({
                "branchName": cleanup.branch_name,
                "path": cleanup.worktree_path,
                "archivedRef": cleanup.archive_ref,
                "archivedOid": cleanup.branch_oid,
                "discardedUncommitted": cleanup.uncommitted,
                "decidedBy": decided_by,
            }),
            attachment: None,
        })?;

        Ok(CleanupRecord {
            id,
            task_id: task_id.to_owned(),
            removed_path: cleanup.worktree_path,
            branch_name: cleanup.branch_name,
            archived_ref: cleanup.branch_oid.as_ref().map(|_| cleanup.archive_ref),
            archived_oid: cleanup.branch_oid,
            state: "completed".into(),
        })
    }

    /// Put back a worktree that was cleaned up, from its archived branch tip.
    pub fn restore(
        &self,
        cleanup_id: &str,
        decided_by: &str,
    ) -> Result<CleanupRecord, TaskWorktreeError> {
        let row = self.storage.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT task_id, removed_path, branch_name, archived_ref, archived_oid, state
                     FROM worktree_cleanups WHERE id = ?1",
                    [cleanup_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, Option<String>>(4)?,
                            row.get::<_, String>(5)?,
                        ))
                    },
                )
                .optional()
        })?;
        let Some((task_id, removed_path, branch_name, archived_ref, archived_oid, state)) = row
        else {
            return Err(TaskWorktreeError::CleanupNotFound(cleanup_id.to_owned()));
        };
        if state != "completed" {
            return Err(TaskWorktreeError::CleanupNotRestorable {
                cleanup_id: cleanup_id.to_owned(),
                state,
            });
        }
        let Some(oid) = archived_oid.clone() else {
            return Err(TaskWorktreeError::CleanupNotRestorable {
                cleanup_id: cleanup_id.to_owned(),
                state: "no archived commit".into(),
            });
        };

        let plan = self.worktrees.get(&task_id)?;
        let repository = self.projects.find_repository(&plan.repository_id)?;
        let path = PathBuf::from(&removed_path);
        run(
            &repository.root_path,
            [
                OsString::from("worktree"),
                OsString::from("add"),
                OsString::from("-b"),
                OsString::from(&branch_name),
                path.clone().into_os_string(),
                OsString::from(&oid),
            ],
        )?;

        self.storage.with_transaction(|transaction| {
            transaction.execute(
                "UPDATE worktree_cleanups SET state = 'restored' WHERE id = ?1",
                [cleanup_id],
            )?;
            transaction.execute(
                "UPDATE branches SET state = 'created', head_oid = ?2,
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE task_id = ?1",
                params![&task_id, &oid],
            )?;
            transaction.execute(
                "UPDATE worktrees SET state = 'ready', last_error = NULL,
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE task_id = ?1",
                [&task_id],
            )?;
            transaction.execute(
                "UPDATE tasks SET status = 'ready',
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE id = ?1",
                [&task_id],
            )?;
            Ok(())
        })?;

        self.record(PlanEvent {
            task_id: &task_id,
            kind: ActivityKind::WorktreeRestored,
            actor: Actor::Human,
            actor_label: decided_by,
            summary: &format!(
                "Restored {branch_name} at {removed_path} from {}",
                short(&oid)
            ),
            payload: json!({
                "branchName": branch_name,
                "path": removed_path,
                "archivedRef": archived_ref,
                "archivedOid": oid,
                "decidedBy": decided_by,
            }),
            attachment: None,
        })?;

        Ok(CleanupRecord {
            id: cleanup_id.to_owned(),
            task_id,
            removed_path: path,
            branch_name,
            archived_ref,
            archived_oid,
            state: "restored".into(),
        })
    }

    fn run_setup_commands(
        &self,
        task_id: &str,
        plan: &WorktreePlan,
        commands: &[SetupCommand],
    ) -> Result<(), TaskWorktreeError> {
        for setup in commands {
            let started = std::time::Instant::now();
            let output = run_shell(&plan.worktree_path, &setup.command);
            let elapsed = started.elapsed();
            let (status, detail) = match &output {
                Ok(text) => ("completed", text.clone()),
                Err(error) => ("failed", error.to_string()),
            };
            self.record(PlanEvent {
                task_id,
                kind: ActivityKind::Command,
                actor: Actor::System,
                actor_label: "mars (setup)",
                summary: &format!("setup: {}", setup.command),
                payload: json!({
                    "command": setup.command,
                    "status": status,
                    "purpose": setup.purpose,
                    "classification": setup.classification,
                    "approvedWithPlan": true,
                }),
                attachment: Some((Attachment::text("text/plain", detail.clone()), elapsed)),
            })?;
            if let Err(error) = output {
                return Err(TaskWorktreeError::SetupCommandFailed {
                    command: setup.command.clone(),
                    detail: error.to_string(),
                });
            }
        }
        Ok(())
    }

    fn assemble(
        &self,
        plan: &WorktreePlan,
        repository: &ProjectRecord,
        base_oid: String,
        request: &PlanRequest,
    ) -> Result<WorktreeProposal, TaskWorktreeError> {
        let disk = report_for(&plan.worktree_path);
        let mut preflight = Vec::new();

        if plan.state == WorktreeState::Planned && plan.worktree_path.exists() {
            preflight.push(PreflightFinding {
                code: PreflightCode::WorktreeExists,
                severity: PreflightSeverity::Blocking,
                detail: format!("{} already exists on disk", plan.worktree_path.display()),
                resolutions: vec![
                    Resolution {
                        action: "Plan a different path".into(),
                        consequence:
                            "Mars picks the next free name; the existing directory is untouched."
                                .into(),
                    },
                    Resolution {
                        action: "Remove the directory yourself, then re-plan".into(),
                        consequence:
                            "Anything in that directory is gone; Mars will not delete it for you."
                                .into(),
                    },
                ],
            });
        }

        if plan.state == WorktreeState::Planned
            && branch_exists(repository, &plan.branch_name).unwrap_or(false)
        {
            preflight.push(PreflightFinding {
                code: PreflightCode::BranchExists,
                severity: PreflightSeverity::Blocking,
                detail: format!("branch {} already exists in Git", plan.branch_name),
                resolutions: vec![Resolution {
                    action: "Plan a different branch name".into(),
                    consequence: "The existing branch and any work on it is left alone.".into(),
                }],
            });
        }

        if let Ok(status) = run(
            &repository.root_path,
            ["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        ) && status_is_dirty(&status.stdout)
        {
            preflight.push(PreflightFinding {
                code: PreflightCode::DirtyBase,
                severity: PreflightSeverity::Warning,
                detail: format!(
                    "{} has uncommitted changes; the new worktree starts from {} and will not see them",
                    repository.root_path.display(),
                    short(&base_oid)
                ),
                resolutions: vec![
                    Resolution {
                        action: "Continue".into(),
                        consequence:
                            "The worktree is cut from the commit, not your working tree. Your \
                             uncommitted changes stay where they are."
                                .into(),
                    },
                    Resolution {
                        action: "Commit or stash first, then re-plan".into(),
                        consequence:
                            "The new base includes that work, so the agent starts from it.".into(),
                    },
                ],
            });
        }

        if disk.low {
            preflight.push(PreflightFinding {
                code: PreflightCode::LowDisk,
                severity: PreflightSeverity::Warning,
                detail: format!(
                    "{} MiB free on the volume holding {}",
                    disk.available_bytes.unwrap_or(0) / (1024 * 1024),
                    plan.worktree_path.display()
                ),
                resolutions: vec![Resolution {
                    action: "Continue anyway".into(),
                    consequence:
                        "A checkout that runs out of space leaves a partial worktree you have to \
                         clean up by hand."
                            .into(),
                }],
            });
        }

        for setup in &request.setup_commands {
            let analysis = classify_command(&setup.command);
            if analysis.class == CommandClass::Destructive {
                preflight.push(PreflightFinding {
                    code: PreflightCode::RiskySetupCommand,
                    severity: PreflightSeverity::Warning,
                    detail: format!("setup command `{}`: {}", setup.command, analysis.reason),
                    resolutions: vec![Resolution {
                        action: "Remove it from the plan".into(),
                        consequence:
                            "Setup runs without it; you can run it yourself in the worktree later."
                                .into(),
                    }],
                });
            }
        }

        let mut proposal = WorktreeProposal {
            task_id: plan.task_id.clone(),
            repository_id: plan.repository_id.clone(),
            title: plan.title.clone(),
            intent: plan.intent.clone(),
            base_ref: plan.base_ref.clone(),
            base_oid,
            branch_name: plan.branch_name.clone(),
            worktree_path: plan.worktree_path.clone(),
            expected_scope: request.expected_scope.clone(),
            setup_commands: request
                .setup_commands
                .iter()
                .map(|setup| SetupCommand::new(&setup.command, &setup.purpose))
                .collect(),
            preflight,
            disk,
            state: plan.state,
            fingerprint: String::new(),
            approved_by: None,
            approved_at: None,
        };
        proposal.fingerprint = fingerprint_of(&proposal);
        Ok(proposal)
    }

    fn resolve_base(
        &self,
        repository: &ProjectRecord,
        base_ref: &str,
        repository_id: &str,
    ) -> Result<String, TaskWorktreeError> {
        match run(
            &repository.root_path,
            [
                OsString::from("rev-parse"),
                OsString::from("--verify"),
                OsString::from("--end-of-options"),
                OsString::from(format!("{base_ref}^{{commit}}")),
            ],
        ) {
            Ok(output) => Ok(output.stdout_text()),
            Err(GitCommandError::Failed { .. }) => Err(TaskWorktreeError::InvalidBaseRef {
                repository_id: repository_id.to_owned(),
                base_ref: base_ref.to_owned(),
            }),
            Err(error) => Err(error.into()),
        }
    }

    fn persist(&self, proposal: &WorktreeProposal) -> Result<(), TaskWorktreeError> {
        let expected_scope = serde_json::to_string(&proposal.expected_scope)?;
        let setup_commands = serde_json::to_string(&proposal.setup_commands)?;
        let preflight = serde_json::to_string(&proposal.preflight)?;
        let disk = serde_json::to_string(&proposal.disk)?;
        self.storage.with_connection(|connection| {
            connection.execute(
                "INSERT INTO worktree_plans
                     (task_id, repository_id, base_ref, base_oid, branch_name, worktree_path,
                      expected_scope_json, setup_commands_json, preflight_json, disk_json,
                      fingerprint, approved_by, approved_at, approved_fingerprint,
                      created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, NULL, NULL,
                         strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                         strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                 ON CONFLICT(task_id) DO UPDATE SET
                     base_oid = excluded.base_oid,
                     expected_scope_json = excluded.expected_scope_json,
                     setup_commands_json = excluded.setup_commands_json,
                     preflight_json = excluded.preflight_json,
                     disk_json = excluded.disk_json,
                     fingerprint = excluded.fingerprint,
                     updated_at = excluded.updated_at",
                params![
                    &proposal.task_id,
                    &proposal.repository_id,
                    &proposal.base_ref,
                    &proposal.base_oid,
                    &proposal.branch_name,
                    proposal.worktree_path.to_string_lossy(),
                    expected_scope,
                    setup_commands,
                    preflight,
                    disk,
                    &proposal.fingerprint,
                ],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    /// Move the plan onto a new base commit. Any approval is dropped, because
    /// the plan a person said yes to no longer exists.
    fn rebase_plan(&self, task_id: &str, base_oid: &str) -> Result<(), TaskWorktreeError> {
        self.storage.with_connection(|connection| {
            connection.execute(
                "UPDATE worktree_plans
                 SET base_oid = ?2, acknowledged_drift_oid = NULL,
                     approved_by = NULL, approved_at = NULL, approved_fingerprint = NULL,
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE task_id = ?1",
                params![task_id, base_oid],
            )?;
            Ok(())
        })?;
        let refreshed = self.current(task_id)?;
        self.persist(&refreshed)
    }

    /// Move the task's durable rows to the removed state after a cleanup.
    fn mark_removed(&self, task_id: &str) -> Result<(), TaskWorktreeError> {
        self.storage.with_transaction(|transaction| {
            transaction.execute(
                "UPDATE branches SET state = 'removed',
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE task_id = ?1",
                [task_id],
            )?;
            transaction.execute(
                "UPDATE worktrees SET state = 'removed', last_error = NULL,
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE task_id = ?1",
                [task_id],
            )?;
            transaction.execute(
                "UPDATE tasks SET status = 'completed',
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE id = ?1",
                [task_id],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    fn finish_cleanup(&self, id: &str, state: &str) -> Result<(), TaskWorktreeError> {
        self.storage.with_connection(|connection| {
            connection.execute(
                "UPDATE worktree_cleanups
                 SET state = ?2, completed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE id = ?1",
                params![id, state],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    fn stored(&self, task_id: &str) -> Result<StoredPlan, TaskWorktreeError> {
        let row = self.storage.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT base_oid, expected_scope_json, setup_commands_json,
                            acknowledged_drift_oid, approved_by, approved_at,
                            approved_fingerprint
                     FROM worktree_plans WHERE task_id = ?1",
                    [task_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, Option<String>>(4)?,
                            row.get::<_, Option<String>>(5)?,
                            row.get::<_, Option<String>>(6)?,
                        ))
                    },
                )
                .optional()
        })?;
        let Some((
            base_oid,
            expected_scope,
            setup_commands,
            acknowledged_drift_oid,
            approved_by,
            approved_at,
            approved_fingerprint,
        )) = row
        else {
            return Err(TaskWorktreeError::PlanNotFound(task_id.to_owned()));
        };
        Ok(StoredPlan {
            base_oid,
            expected_scope: serde_json::from_str(&expected_scope)?,
            setup_commands: serde_json::from_str(&setup_commands)?,
            acknowledged_drift_oid,
            approved_by,
            approved_at,
            approved_fingerprint,
        })
    }

    fn record(&self, event: PlanEvent<'_>) -> Result<(), TaskWorktreeError> {
        let Some(activity) = self.activity.as_ref() else {
            return Ok(());
        };
        let mut entry = ActivityEntry::new(
            event.task_id,
            event.kind,
            event.actor,
            event.actor_label,
            event.summary,
        )
        .with_payload(event.payload);
        if let Some((attachment, duration)) = event.attachment {
            entry = entry.with_attachment(attachment).lasting(duration);
        }
        activity
            .append(entry)
            .map_err(|error| TaskWorktreeError::Activity(error.to_string()))?;
        Ok(())
    }
}

/// One activity event a plan step wants to record.
struct PlanEvent<'a> {
    task_id: &'a str,
    kind: ActivityKind,
    actor: Actor,
    actor_label: &'a str,
    summary: &'a str,
    payload: serde_json::Value,
    attachment: Option<(Attachment, std::time::Duration)>,
}

struct StoredPlan {
    base_oid: String,
    expected_scope: Vec<String>,
    setup_commands: Vec<SetupCommand>,
    acknowledged_drift_oid: Option<String>,
    approved_by: Option<String>,
    approved_at: Option<String>,
    approved_fingerprint: Option<String>,
}

fn blocking_detail(proposal: &WorktreeProposal) -> String {
    proposal
        .preflight
        .iter()
        .filter(|finding| finding.severity == PreflightSeverity::Blocking)
        .map(|finding| finding.detail.clone())
        .collect::<Vec<_>>()
        .join("; ")
}

fn drift_options(state: WorktreeState) -> Vec<DriftOption> {
    if state == WorktreeState::Planned {
        return vec![
            DriftOption {
                choice: DriftChoice::Recreate,
                label: "Plan again from the new base".into(),
                consequence:
                    "Nothing exists yet, so the plan simply moves to the new commit. It needs \
                     approving again before anything is created."
                        .into(),
            },
            DriftOption {
                choice: DriftChoice::ContinueAndDiverge,
                label: "Keep the base I approved".into(),
                consequence:
                    "The worktree is cut from the older commit and will not contain the newer \
                     work, so a merge later has to reconcile both."
                        .into(),
            },
        ];
    }
    vec![
        DriftOption {
            choice: DriftChoice::Rebase,
            label: "Rebase onto the new base".into(),
            consequence:
                "Commits already made on this branch are replayed onto the newer commit. Conflicts \
                 stop the rebase and leave the worktree mid-rebase for you to finish."
                    .into(),
        },
        DriftOption {
            choice: DriftChoice::ContinueAndDiverge,
            label: "Continue from the old base".into(),
            consequence:
                "Nothing changes now. The branch keeps diverging, and whoever merges it later \
                 resolves the difference."
                    .into(),
        },
        DriftOption {
            choice: DriftChoice::Recreate,
            label: "Throw the branch away and start again".into(),
            consequence:
                "Every commit on this branch is discarded and a fresh branch is cut from the new \
                 base. Only available before the worktree exists."
                    .into(),
        },
    ]
}

fn branch_exists(repository: &ProjectRecord, branch: &str) -> Result<bool, GitCommandError> {
    let reference = format!("refs/heads/{branch}");
    match run(
        &repository.root_path,
        ["show-ref", "--verify", "--quiet", &reference],
    ) {
        Ok(_) => Ok(true),
        Err(GitCommandError::Failed {
            exit_code: Some(1), ..
        }) => Ok(false),
        Err(error) => Err(error),
    }
}

fn parse_status_paths(stdout: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(stdout)
        .split('\0')
        .filter(|entry| entry.len() > 3)
        .map(|entry| entry[3..].to_owned())
        .collect()
}

fn run_shell(cwd: &Path, command: &str) -> Result<String, GitCommandError> {
    let (program, flag) = if cfg!(windows) {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    };
    let output = mars_core::process::command(program)
        .arg(flag)
        .arg(command)
        .current_dir(cwd)
        .output()
        .map_err(|source| GitCommandError::Launch {
            cwd: cwd.display().to_string(),
            source,
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if output.status.success() {
        Ok(format!("{stdout}{stderr}"))
    } else {
        Err(GitCommandError::Failed {
            exit_code: output.status.code(),
            stderr: format!("{stdout}{stderr}").trim().to_owned(),
        })
    }
}

/// A digest over what the plan decides, not over what it observed.
///
/// Free space and preflight findings are readings of a world that moves on its
/// own; including them would invalidate an approval every time the disk filled
/// slightly. What must not change silently is the base commit, the branch, the
/// path, the file scope, and the setup commands — so those are what is hashed,
/// and `approve` and `provision_approved` re-check the findings separately.
fn fingerprint_of<T: Serialize>(value: &T) -> String {
    let mut canonical = serde_json::to_value(value).unwrap_or(serde_json::Value::Null);
    if let Some(object) = canonical.as_object_mut() {
        for field in [
            "fingerprint",
            "approvedBy",
            "approvedAt",
            "disk",
            "preflight",
        ] {
            object.remove(field);
        }
    }
    let mut hasher = Sha256::new();
    hasher.update(canonical.to_string().as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn short(oid: &str) -> String {
    oid.chars().take(9).collect()
}
