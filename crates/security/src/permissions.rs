use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use mars_storage::Storage;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    ReadFiles,
    WriteFiles,
    RunCommand,
    Network,
    MutateGit,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum PermissionScope {
    Repository { repository_id: String },
    Worktree { task_id: String, root: PathBuf },
    Command { program: String, cwd: PathBuf },
    NetworkHost { host: String },
    Remote { remote: String, branch: String },
}

impl PermissionScope {
    /// Whether a grant over `self` covers a request over `child`. Containment
    /// only ever narrows: a broader scope admits a narrower one, never the
    /// reverse, and a lexical `..` in either path disqualifies both.
    #[must_use]
    pub fn contains(&self, child: &Self) -> bool {
        match (self, child) {
            (
                Self::Repository { repository_id },
                Self::Repository {
                    repository_id: child_id,
                },
            ) => repository_id == child_id,
            (
                Self::Worktree { task_id, root },
                Self::Worktree {
                    task_id: child_task,
                    root: child_root,
                },
            ) => task_id == child_task && safe_descendant(root, child_root),
            (
                Self::Command { program, cwd },
                Self::Command {
                    program: child_program,
                    cwd: child_cwd,
                },
            ) => program == child_program && safe_descendant(cwd, child_cwd),
            (Self::NetworkHost { host }, Self::NetworkHost { host: child_host }) => {
                host == child_host
            }
            (
                Self::Remote { remote, branch },
                Self::Remote {
                    remote: child_remote,
                    branch: child_branch,
                },
            ) => remote == child_remote && branch == child_branch,
            _ => false,
        }
    }
}

fn safe_descendant(parent: &Path, child: &Path) -> bool {
    parent.is_absolute()
        && child.is_absolute()
        && parent
            .components()
            .all(|component| !matches!(component, Component::CurDir | Component::ParentDir))
        && child
            .components()
            .all(|component| !matches!(component, Component::CurDir | Component::ParentDir))
        && child.starts_with(parent)
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequest {
    pub id: String,
    pub capability: Capability,
    pub scope: PermissionScope,
}

impl PermissionRequest {
    #[must_use]
    pub fn new(capability: Capability, scope: PermissionScope) -> Self {
        Self {
            id: Uuid::now_v7().to_string(),
            capability,
            scope,
        }
    }

    fn key(&self) -> PermissionKey {
        PermissionKey {
            capability: self.capability,
            scope: self.scope.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    Deny,
    Ask,
    AllowOnce,
    AllowSession,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionGrant {
    capability: Capability,
    scope: PermissionScope,
}

impl PermissionGrant {
    #[must_use]
    pub fn allows(&self, request: &PermissionRequest) -> bool {
        self.capability == request.capability && self.scope.contains(&request.scope)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PermissionOutcome {
    Denied,
    Ask,
    Allowed(PermissionGrant),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct PermissionKey {
    capability: Capability,
    scope: PermissionScope,
}

#[derive(Default)]
struct PermissionState {
    denied: HashSet<PermissionKey>,
    once: HashMap<PermissionKey, usize>,
    session: HashSet<PermissionKey>,
}

#[derive(Debug, Error)]
pub enum PermissionError {
    #[error(transparent)]
    Storage(#[from] mars_storage::StorageError),
    #[error("failed to encode permission audit data: {0}")]
    Json(#[from] serde_json::Error),
    #[error("permission state lock was poisoned")]
    StatePoisoned,
    #[error("permission request `{0}` does not exist")]
    RequestNotFound(String),
    #[error("permission request `{0}` has already been decided")]
    RequestAlreadyDecided(String),
    #[error("permission grant `{0}` does not exist or is already revoked")]
    GrantNotFound(String),
    #[error("failed to record a permission decision in the activity log: {0}")]
    Activity(String),
}

pub struct PermissionAudit {
    engine: PermissionEngine,
    storage: Arc<Storage>,
}

impl PermissionAudit {
    #[must_use]
    pub fn new(storage: Arc<Storage>) -> Self {
        Self {
            engine: PermissionEngine::new(),
            storage,
        }
    }

    pub fn record(
        &self,
        request: &PermissionRequest,
        decision: PermissionDecision,
    ) -> Result<PermissionOutcome, PermissionError> {
        let scope = serde_json::to_string(&request.scope)?;
        self.storage.with_connection(|connection| {
            connection.execute(
                "INSERT INTO permission_decisions
                 (id, session_id, request_id, capability, scope_json, decision, decided_at)
                 VALUES (?1, NULL, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                (
                    format!("decision:{}", request.id),
                    &request.id,
                    capability_name(request.capability),
                    scope,
                    decision_name(decision),
                ),
            )?;
            Ok(())
        })?;
        self.engine.record_decision(request, decision)?;
        self.engine.evaluate(request, None)
    }
}

fn capability_name(capability: Capability) -> &'static str {
    match capability {
        Capability::ReadFiles => "read_files",
        Capability::WriteFiles => "write_files",
        Capability::RunCommand => "run_command",
        Capability::Network => "network",
        Capability::MutateGit => "mutate_git",
    }
}

fn decision_name(decision: PermissionDecision) -> &'static str {
    match decision {
        PermissionDecision::Deny => "deny",
        PermissionDecision::Ask => "ask",
        PermissionDecision::AllowOnce => "allow_once",
        PermissionDecision::AllowSession => "allow_session",
    }
}

#[derive(Default)]
pub struct PermissionEngine {
    state: Mutex<PermissionState>,
}

impl PermissionEngine {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_decision(
        &self,
        request: &PermissionRequest,
        decision: PermissionDecision,
    ) -> Result<(), PermissionError> {
        let key = request.key();
        let mut state = self.state()?;
        state.denied.remove(&key);
        state.once.remove(&key);
        state.session.remove(&key);
        match decision {
            PermissionDecision::Deny => {
                state.denied.insert(key);
            }
            PermissionDecision::Ask => {}
            PermissionDecision::AllowOnce => {
                state.once.insert(key, 1);
            }
            PermissionDecision::AllowSession => {
                state.session.insert(key);
            }
        }
        Ok(())
    }

    pub fn evaluate(
        &self,
        request: &PermissionRequest,
        parent: Option<&PermissionGrant>,
    ) -> Result<PermissionOutcome, PermissionError> {
        if parent.is_some_and(|grant| !grant.allows(request)) {
            return Ok(PermissionOutcome::Denied);
        }
        let key = request.key();
        let mut state = self.state()?;
        if state.denied.contains(&key) {
            return Ok(PermissionOutcome::Denied);
        }
        if let Some(remaining) = state.once.get_mut(&key) {
            *remaining -= 1;
            let grant = grant(request);
            if *remaining == 0 {
                state.once.remove(&key);
            }
            return Ok(PermissionOutcome::Allowed(grant));
        }
        if state.session.contains(&key) {
            return Ok(PermissionOutcome::Allowed(grant(request)));
        }
        Ok(PermissionOutcome::Ask)
    }

    fn state(&self) -> Result<MutexGuard<'_, PermissionState>, PermissionError> {
        self.state
            .lock()
            .map_err(|_| PermissionError::StatePoisoned)
    }
}

fn grant(request: &PermissionRequest) -> PermissionGrant {
    PermissionGrant {
        capability: request.capability,
        scope: request.scope.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        Capability, PermissionDecision, PermissionEngine, PermissionOutcome, PermissionRequest,
        PermissionScope,
    };

    /// Scope containment is defined over absolute paths, and what counts as
    /// absolute differs by platform, so the fixtures are rooted rather than
    /// written as POSIX literals.
    fn absolute(path: &str) -> PathBuf {
        let mut rooted = if cfg!(windows) {
            PathBuf::from("C:\\")
        } else {
            PathBuf::from("/")
        };
        for segment in path.split('/') {
            rooted.push(segment);
        }
        rooted
    }

    fn write_request(path: &str) -> PermissionRequest {
        PermissionRequest::new(
            Capability::WriteFiles,
            PermissionScope::Worktree {
                task_id: "task-1".into(),
                root: absolute(path),
            },
        )
    }

    #[test]
    fn deny_ask_once_and_session_decisions_follow_their_scope() {
        let engine = PermissionEngine::new();
        let request = write_request("worktree/src");
        assert_eq!(
            engine
                .evaluate(&request, None)
                .expect("policy should evaluate"),
            PermissionOutcome::Ask
        );

        engine
            .record_decision(&request, PermissionDecision::Deny)
            .expect("deny should record");
        assert_eq!(
            engine
                .evaluate(&request, None)
                .expect("deny should evaluate"),
            PermissionOutcome::Denied
        );

        engine
            .record_decision(&request, PermissionDecision::AllowOnce)
            .expect("once should record");
        assert!(matches!(
            engine.evaluate(&request, None).expect("once should allow"),
            PermissionOutcome::Allowed(_)
        ));
        assert_eq!(
            engine.evaluate(&request, None).expect("once should expire"),
            PermissionOutcome::Ask
        );

        engine
            .record_decision(&request, PermissionDecision::AllowSession)
            .expect("session should record");
        assert!(matches!(
            engine
                .evaluate(&request, None)
                .expect("session should allow"),
            PermissionOutcome::Allowed(_)
        ));
        assert!(matches!(
            engine
                .evaluate(&request, None)
                .expect("session should persist"),
            PermissionOutcome::Allowed(_)
        ));
    }

    #[test]
    fn child_requests_can_narrow_but_never_widen_the_parent_grant() {
        let engine = PermissionEngine::new();
        let parent_request = write_request("worktree");
        engine
            .record_decision(&parent_request, PermissionDecision::AllowSession)
            .expect("parent grant should record");
        let PermissionOutcome::Allowed(parent) = engine
            .evaluate(&parent_request, None)
            .expect("parent should evaluate")
        else {
            panic!("parent should be allowed")
        };

        let child = write_request("worktree/src/main.ts");
        engine
            .record_decision(&child, PermissionDecision::AllowSession)
            .expect("child grant should record");
        assert!(matches!(
            engine
                .evaluate(&child, Some(&parent))
                .expect("child should evaluate"),
            PermissionOutcome::Allowed(_)
        ));

        let outside = write_request("other-repository");
        engine
            .record_decision(&outside, PermissionDecision::AllowSession)
            .expect("outside grant should record");
        assert_eq!(
            engine
                .evaluate(&outside, Some(&parent))
                .expect("outside should deny"),
            PermissionOutcome::Denied
        );

        let lexical_escape = write_request("worktree/../other-repository");
        engine
            .record_decision(&lexical_escape, PermissionDecision::AllowSession)
            .expect("escape grant should record");
        assert_eq!(
            engine
                .evaluate(&lexical_escape, Some(&parent))
                .expect("lexical escape should deny"),
            PermissionOutcome::Denied
        );

        let command = PermissionRequest::new(
            Capability::RunCommand,
            PermissionScope::Command {
                program: "git".into(),
                cwd: absolute("worktree"),
            },
        );
        engine
            .record_decision(&command, PermissionDecision::AllowSession)
            .expect("command grant should record");
        assert_eq!(
            engine
                .evaluate(&command, Some(&parent))
                .expect("widening should deny"),
            PermissionOutcome::Denied
        );
    }
}
