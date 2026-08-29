use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mars_storage::projects::{ProjectRecord, ProjectRepository};
use mars_storage::{Storage, StorageError};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use serde_json::{Value, json};
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::command::{GitCommandError, run};
use crate::status::{is_registered, parse_worktree_porcelain_z, status_is_dirty};

type WorktreeRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeState {
    Planned,
    Provisioning,
    Ready,
    Removing,
    Removed,
    Orphaned,
    Failed,
}

impl WorktreeState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Provisioning => "provisioning",
            Self::Ready => "ready",
            Self::Removing => "removing",
            Self::Removed => "removed",
            Self::Orphaned => "orphaned",
            Self::Failed => "failed",
        }
    }

    fn parse(value: &str) -> Result<Self, TaskWorktreeError> {
        match value {
            "planned" => Ok(Self::Planned),
            "provisioning" => Ok(Self::Provisioning),
            "ready" => Ok(Self::Ready),
            "removing" => Ok(Self::Removing),
            "removed" => Ok(Self::Removed),
            "orphaned" => Ok(Self::Orphaned),
            "failed" => Ok(Self::Failed),
            other => Err(TaskWorktreeError::InvalidStoredState(other.to_owned())),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreePlan {
    pub task_id: String,
    pub repository_id: String,
    pub title: String,
    pub intent: String,
    pub branch_name: String,
    pub base_ref: String,
    pub worktree_path: PathBuf,
    pub state: WorktreeState,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationRecord {
    pub id: String,
    pub operation: String,
    pub status: String,
    pub details: Value,
    pub started_at: String,
    pub finished_at: Option<String>,
}

#[derive(Debug, Error)]
pub enum TaskWorktreeError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Git(#[from] GitCommandError),
    #[error("task `{0}` does not exist")]
    TaskNotFound(String),
    #[error("task `{task_id}` cannot transition from {state} via {operation}")]
    InvalidTransition {
        task_id: String,
        state: String,
        operation: &'static str,
    },
    #[error("base ref `{base_ref}` is not a commit in repository `{repository_id}`")]
    InvalidBaseRef {
        repository_id: String,
        base_ref: String,
    },
    #[error("worktree contains uncommitted changes: {path}")]
    DirtyWorktree { path: PathBuf },
    #[error("worktree branch contains commits not reachable from its base: {branch_name}")]
    UnmergedBranch { branch_name: String },
    #[error("planned worktree path already exists: {0}")]
    PathCollision(PathBuf),
    #[error("stored worktree state is invalid: {0}")]
    InvalidStoredState(String),
    #[error("filesystem operation failed for {path}: {source}")]
    Filesystem {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to format a timestamp: {0}")]
    Timestamp(#[from] time::error::Format),
    #[error("failed to encode mutation evidence: {0}")]
    Json(#[from] serde_json::Error),
}

pub struct TaskWorktreeService {
    storage: Arc<Storage>,
    projects: ProjectRepository,
    workspace_root: PathBuf,
}

impl TaskWorktreeService {
    #[must_use]
    pub fn new(storage: Arc<Storage>, workspace_root: impl AsRef<Path>) -> Self {
        Self {
            projects: ProjectRepository::new(storage.clone()),
            storage,
            workspace_root: workspace_root.as_ref().to_owned(),
        }
    }

    pub fn create_task(
        &self,
        repository_id: &str,
        title: &str,
        intent: &str,
        base_ref: &str,
    ) -> Result<WorktreePlan, TaskWorktreeError> {
        let repository = self.projects.find_repository(repository_id)?;
        let existing_branches = task_branches(&repository)?;
        let stem = slug(title);
        let timestamp = now()?;
        let task_id = Uuid::now_v7().to_string();
        let branch_id = Uuid::now_v7().to_string();
        let worktree_id = Uuid::now_v7().to_string();

        let plan = self.storage.with_transaction(|transaction| {
            let mut suffix = 1_usize;
            let (branch_name, worktree_path) = loop {
                let candidate = if suffix == 1 {
                    stem.clone()
                } else {
                    format!("{stem}-{suffix}")
                };
                let branch_name = format!("mars/{candidate}");
                let worktree_path = self.workspace_root.join(&candidate);
                let occupied: bool = transaction.query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM branches WHERE repository_id = ?1 AND name = ?2
                        UNION ALL SELECT 1 FROM worktrees WHERE path = ?3
                    )",
                    params![repository_id, branch_name, worktree_path.to_string_lossy()],
                    |row| row.get(0),
                )?;
                if !occupied && !worktree_path.exists() && !existing_branches.contains(&branch_name)
                {
                    break (branch_name, worktree_path);
                }
                suffix += 1;
            };
            transaction.execute(
                "INSERT INTO tasks
                 (id, project_id, repository_id, title, intent, status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'planned', ?6, ?6)",
                params![
                    task_id,
                    repository.project_id,
                    repository_id,
                    title,
                    intent,
                    timestamp
                ],
            )?;
            transaction.execute(
                "INSERT INTO branches
                 (id, repository_id, task_id, name, base_ref, state, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'planned', ?6, ?6)",
                params![
                    branch_id,
                    repository_id,
                    task_id,
                    branch_name,
                    base_ref,
                    timestamp
                ],
            )?;
            transaction.execute(
                "INSERT INTO worktrees
                 (id, repository_id, task_id, branch_id, path, state, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'planned', ?6, ?6)",
                params![
                    worktree_id,
                    repository_id,
                    task_id,
                    branch_id,
                    worktree_path.to_string_lossy(),
                    timestamp
                ],
            )?;
            Ok(WorktreePlan {
                task_id: task_id.clone(),
                repository_id: repository_id.to_owned(),
                title: title.to_owned(),
                intent: intent.to_owned(),
                branch_name,
                base_ref: base_ref.to_owned(),
                worktree_path,
                state: WorktreeState::Planned,
            })
        })?;
        Ok(plan)
    }

    pub fn provision(&self, task_id: &str) -> Result<WorktreePlan, TaskWorktreeError> {
        let plan = self.get(task_id)?;
        if plan.state == WorktreeState::Ready {
            return Ok(plan);
        }
        ensure_transition(&plan, WorktreeState::Planned, "provision")?;
        if plan.worktree_path.exists() {
            return Err(TaskWorktreeError::PathCollision(plan.worktree_path));
        }
        let repository = self.projects.find_repository(&plan.repository_id)?;
        let base_oid = match resolve_base(&repository, &plan.base_ref) {
            Ok(base_oid) => base_oid,
            Err(GitCommandError::Failed { .. }) => {
                return Err(TaskWorktreeError::InvalidBaseRef {
                    repository_id: plan.repository_id.clone(),
                    base_ref: plan.base_ref.clone(),
                });
            }
            Err(error) => return Err(error.into()),
        };
        if branch_exists(&repository, &plan.branch_name)? {
            return Err(TaskWorktreeError::InvalidTransition {
                task_id: task_id.to_owned(),
                state: plan.state.as_str().to_owned(),
                operation: "provision over existing branch",
            });
        }
        self.set_state(task_id, WorktreeState::Provisioning, None)?;

        let branch_mutation = self.start_mutation(
            &plan,
            "create_branch",
            json!({ "branch": plan.branch_name, "baseOid": base_oid }),
        )?;
        if let Err(error) = run(
            &repository.root_path,
            [
                OsString::from("branch"),
                OsString::from(&plan.branch_name),
                OsString::from(&base_oid),
            ],
        ) {
            self.finish_mutation(&branch_mutation, "failed")?;
            self.set_state(task_id, WorktreeState::Failed, Some(&error.to_string()))?;
            return Err(error.into());
        }
        self.finish_mutation(&branch_mutation, "completed")?;

        let add_result = self.add_worktree(&repository, &plan);
        if let Err(error) = add_result {
            let rollback = run(
                &repository.root_path,
                [
                    OsString::from("branch"),
                    OsString::from("-D"),
                    OsString::from(&plan.branch_name),
                ],
            );
            let rollback_status = if rollback.is_ok() {
                "rolled_back"
            } else {
                "failed"
            };
            self.finish_mutation(&branch_mutation, rollback_status)?;
            let failure = match rollback {
                Ok(_) => error.to_string(),
                Err(rollback_error) => {
                    format!("{error}; branch rollback also failed: {rollback_error}")
                }
            };
            self.set_state(task_id, WorktreeState::Failed, Some(&failure))?;
            return Err(error);
        }

        self.storage.with_transaction(|transaction| {
            let timestamp = now_sql()?;
            transaction.execute(
                "UPDATE branches SET state = 'created', head_oid = ?2, updated_at = ?3 WHERE task_id = ?1",
                params![task_id, base_oid, timestamp],
            )?;
            transaction.execute(
                "UPDATE worktrees SET state = 'ready', last_error = NULL, updated_at = ?2 WHERE task_id = ?1",
                params![task_id, timestamp],
            )?;
            transaction.execute(
                "UPDATE tasks SET status = 'ready', updated_at = ?2 WHERE id = ?1",
                params![task_id, timestamp],
            )?;
            Ok(())
        })?;
        self.get(task_id)
    }

    pub fn remove(&self, task_id: &str) -> Result<WorktreePlan, TaskWorktreeError> {
        let plan = self.get(task_id)?;
        if plan.state == WorktreeState::Removed {
            return Ok(plan);
        }
        ensure_transition(&plan, WorktreeState::Ready, "remove")?;
        let repository = self.projects.find_repository(&plan.repository_id)?;
        let status = run(
            &plan.worktree_path,
            ["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        )?;
        if status_is_dirty(&status.stdout) {
            return Err(TaskWorktreeError::DirtyWorktree {
                path: plan.worktree_path,
            });
        }
        let base_oid = self.base_oid(task_id)?;
        let range = format!("{base_oid}..{}", plan.branch_name);
        let ahead = run(&repository.root_path, ["rev-list", "--count", &range])?
            .stdout_text()
            .parse::<usize>()
            .unwrap_or(0);
        if ahead > 0 {
            return Err(TaskWorktreeError::UnmergedBranch {
                branch_name: plan.branch_name,
            });
        }
        self.set_state(task_id, WorktreeState::Removing, None)?;
        let remove_mutation = self.start_mutation(
            &plan,
            "remove_worktree",
            json!({ "path": plan.worktree_path }),
        )?;
        if let Err(error) = run(
            &repository.root_path,
            [
                OsString::from("worktree"),
                OsString::from("remove"),
                plan.worktree_path.clone().into_os_string(),
            ],
        ) {
            self.finish_mutation(&remove_mutation, "failed")?;
            self.set_state(task_id, WorktreeState::Ready, Some(&error.to_string()))?;
            return Err(error.into());
        }
        self.finish_mutation(&remove_mutation, "completed")?;

        let branch_mutation = self.start_mutation(
            &plan,
            "delete_branch",
            json!({ "branch": plan.branch_name }),
        )?;
        if let Err(error) = run(&repository.root_path, ["branch", "-D", &plan.branch_name]) {
            self.finish_mutation(&branch_mutation, "failed")?;
            self.set_state(task_id, WorktreeState::Orphaned, Some(&error.to_string()))?;
            return Err(error.into());
        }
        self.finish_mutation(&branch_mutation, "completed")?;
        self.storage.with_transaction(|transaction| {
            let timestamp = now_sql()?;
            transaction.execute(
                "UPDATE branches SET state = 'removed', updated_at = ?2 WHERE task_id = ?1",
                params![task_id, timestamp],
            )?;
            transaction.execute(
                "UPDATE worktrees SET state = 'removed', last_error = NULL, updated_at = ?2 WHERE task_id = ?1",
                params![task_id, timestamp],
            )?;
            transaction.execute(
                "UPDATE tasks SET status = 'completed', updated_at = ?2 WHERE id = ?1",
                params![task_id, timestamp],
            )?;
            Ok(())
        })?;
        self.get(task_id)
    }

    pub fn reconcile(&self, task_id: &str) -> Result<WorktreePlan, TaskWorktreeError> {
        let plan = self.get(task_id)?;
        if !matches!(
            plan.state,
            WorktreeState::Ready | WorktreeState::Provisioning
        ) {
            return Ok(plan);
        }
        let repository = self.projects.find_repository(&plan.repository_id)?;
        let listed = run(
            &repository.root_path,
            ["worktree", "list", "--porcelain", "-z"],
        )?;
        let registered = parse_worktree_porcelain_z(&listed.stdout);
        if !plan.worktree_path.is_dir() || !is_registered(&registered, &plan.worktree_path) {
            self.set_state(
                task_id,
                WorktreeState::Orphaned,
                Some("worktree path or Git registration is missing"),
            )?;
        }
        self.get(task_id)
    }

    pub fn get(&self, task_id: &str) -> Result<WorktreePlan, TaskWorktreeError> {
        let row = self.storage.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT t.id, t.repository_id, t.title, t.intent, b.name, b.base_ref, w.path, w.state
                     FROM tasks t JOIN branches b ON b.task_id = t.id JOIN worktrees w ON w.task_id = t.id
                     WHERE t.id = ?1",
                    [task_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, String>(6)?,
                            row.get::<_, String>(7)?,
                        ))
                    },
                )
                .optional()
        })?;
        let Some((task_id, repository_id, title, intent, branch_name, base_ref, path, state)) = row
        else {
            return Err(TaskWorktreeError::TaskNotFound(task_id.to_owned()));
        };
        Ok(WorktreePlan {
            task_id,
            repository_id,
            title,
            intent,
            branch_name,
            base_ref,
            worktree_path: PathBuf::from(path),
            state: WorktreeState::parse(&state)?,
        })
    }

    pub fn list(&self, repository_id: &str) -> Result<Vec<WorktreePlan>, TaskWorktreeError> {
        let rows: Vec<WorktreeRow> = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT t.id, t.repository_id, t.title, t.intent, b.name, b.base_ref, w.path, w.state
                 FROM tasks t JOIN branches b ON b.task_id = t.id JOIN worktrees w ON w.task_id = t.id
                 WHERE t.repository_id = ?1 ORDER BY t.created_at DESC, t.id DESC",
            )?;
            statement
                .query_map([repository_id], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                })?
                .collect()
        })?;
        rows.into_iter()
            .map(
                |(task_id, repository_id, title, intent, branch_name, base_ref, path, state)| {
                    Ok(WorktreePlan {
                        task_id,
                        repository_id,
                        title,
                        intent,
                        branch_name,
                        base_ref,
                        worktree_path: PathBuf::from(path),
                        state: WorktreeState::parse(&state)?,
                    })
                },
            )
            .collect()
    }

    pub fn mutation_journal(
        &self,
        task_id: &str,
    ) -> Result<Vec<MutationRecord>, TaskWorktreeError> {
        let raw: Vec<(String, String, String, String, String, Option<String>)> =
            self.storage.with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, operation, status, details_json, started_at, finished_at
                     FROM git_mutations WHERE task_id = ?1 ORDER BY started_at, id",
                )?;
                statement
                    .query_map([task_id], |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                        ))
                    })?
                    .collect()
            })?;
        raw.into_iter()
            .map(
                |(id, operation, status, details, started_at, finished_at)| {
                    Ok(MutationRecord {
                        id,
                        operation,
                        status,
                        details: serde_json::from_str(&details)?,
                        started_at,
                        finished_at,
                    })
                },
            )
            .collect()
    }

    fn add_worktree(
        &self,
        repository: &ProjectRecord,
        plan: &WorktreePlan,
    ) -> Result<(), TaskWorktreeError> {
        fs::create_dir_all(&self.workspace_root).map_err(|source| {
            TaskWorktreeError::Filesystem {
                path: self.workspace_root.clone(),
                source,
            }
        })?;
        let mutation = self.start_mutation(
            plan,
            "add_worktree",
            json!({ "path": plan.worktree_path, "branch": plan.branch_name }),
        )?;
        let result = run(
            &repository.root_path,
            [
                OsString::from("worktree"),
                OsString::from("add"),
                OsString::from("--"),
                plan.worktree_path.clone().into_os_string(),
                OsString::from(&plan.branch_name),
            ],
        );
        match result {
            Ok(_) => {
                self.finish_mutation(&mutation, "completed")?;
                Ok(())
            }
            Err(error) => {
                self.finish_mutation(&mutation, "failed")?;
                Err(error.into())
            }
        }
    }

    fn base_oid(&self, task_id: &str) -> Result<String, TaskWorktreeError> {
        self.storage
            .with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT head_oid FROM branches WHERE task_id = ?1",
                        [task_id],
                        |row| row.get::<_, Option<String>>(0),
                    )
                    .optional()
            })?
            .flatten()
            .ok_or_else(|| {
                TaskWorktreeError::InvalidStoredState(format!(
                    "task `{task_id}` has no recorded base commit"
                ))
            })
    }

    fn set_state(
        &self,
        task_id: &str,
        state: WorktreeState,
        error: Option<&str>,
    ) -> Result<(), TaskWorktreeError> {
        let task_status = match state {
            WorktreeState::Planned => "planned",
            WorktreeState::Provisioning => "provisioning",
            WorktreeState::Ready => "ready",
            WorktreeState::Removed => "completed",
            WorktreeState::Removing | WorktreeState::Orphaned | WorktreeState::Failed => "failed",
        };
        self.storage.with_transaction(|transaction| {
            let timestamp = now_sql()?;
            transaction.execute(
                "UPDATE worktrees SET state = ?2, last_error = ?3, updated_at = ?4 WHERE task_id = ?1",
                params![task_id, state.as_str(), error, timestamp],
            )?;
            transaction.execute(
                "UPDATE tasks SET status = ?2, updated_at = ?3 WHERE id = ?1",
                params![task_id, task_status, timestamp],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    fn start_mutation(
        &self,
        plan: &WorktreePlan,
        operation: &str,
        details: Value,
    ) -> Result<String, TaskWorktreeError> {
        let id = Uuid::now_v7().to_string();
        let timestamp = now()?;
        let details = serde_json::to_string(&details)?;
        self.storage.with_connection(|connection| {
            connection.execute(
                "INSERT INTO git_mutations
                 (id, repository_id, task_id, operation, status, details_json, started_at)
                 VALUES (?1, ?2, ?3, ?4, 'started', ?5, ?6)",
                params![
                    id,
                    plan.repository_id,
                    plan.task_id,
                    operation,
                    details,
                    timestamp
                ],
            )?;
            Ok(())
        })?;
        Ok(id)
    }

    fn finish_mutation(&self, id: &str, status: &str) -> Result<(), TaskWorktreeError> {
        let timestamp = now()?;
        self.storage.with_connection(|connection| {
            connection.execute(
                "UPDATE git_mutations SET status = ?2, finished_at = ?3 WHERE id = ?1",
                params![id, status, timestamp],
            )?;
            Ok(())
        })?;
        Ok(())
    }
}

fn ensure_transition(
    plan: &WorktreePlan,
    expected: WorktreeState,
    operation: &'static str,
) -> Result<(), TaskWorktreeError> {
    if plan.state != expected {
        return Err(TaskWorktreeError::InvalidTransition {
            task_id: plan.task_id.clone(),
            state: plan.state.as_str().to_owned(),
            operation,
        });
    }
    Ok(())
}

fn resolve_base(repository: &ProjectRecord, base_ref: &str) -> Result<String, GitCommandError> {
    run(
        &repository.root_path,
        [
            OsString::from("rev-parse"),
            OsString::from("--verify"),
            OsString::from("--end-of-options"),
            OsString::from(format!("{base_ref}^{{commit}}")),
        ],
    )
    .map(|output| output.stdout_text())
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

fn task_branches(repository: &ProjectRecord) -> Result<HashSet<String>, GitCommandError> {
    let output = run(
        &repository.root_path,
        [
            "for-each-ref",
            "--format=%(refname:short)",
            "refs/heads/mars/",
        ],
    )?;
    Ok(output.stdout_text().lines().map(str::to_owned).collect())
}

fn slug(title: &str) -> String {
    let mut result = String::new();
    let mut separator = false;
    for character in title.trim().chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            result.push(character);
            separator = false;
        } else if !result.is_empty() && !separator {
            result.push('-');
            separator = true;
        }
    }
    while result.ends_with('-') {
        result.pop();
    }
    if result.is_empty() {
        "task".to_owned()
    } else {
        result
            .chars()
            .take(48)
            .collect::<String>()
            .trim_end_matches('-')
            .to_owned()
    }
}

fn now() -> Result<String, time::error::Format> {
    OffsetDateTime::now_utc().format(&Rfc3339)
}

fn now_sql() -> rusqlite::Result<String> {
    now().map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
}
