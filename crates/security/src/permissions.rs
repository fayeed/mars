use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

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
}

impl PermissionScope {
    fn contains(&self, child: &Self) -> bool {
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
    #[error("permission state lock was poisoned")]
    StatePoisoned,
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

    fn write_request(path: &str) -> PermissionRequest {
        PermissionRequest::new(
            Capability::WriteFiles,
            PermissionScope::Worktree {
                task_id: "task-1".into(),
                root: PathBuf::from(path),
            },
        )
    }

    #[test]
    fn deny_ask_once_and_session_decisions_follow_their_scope() {
        let engine = PermissionEngine::new();
        let request = write_request("/worktree/src");
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
        let parent_request = write_request("/worktree");
        engine
            .record_decision(&parent_request, PermissionDecision::AllowSession)
            .expect("parent grant should record");
        let PermissionOutcome::Allowed(parent) = engine
            .evaluate(&parent_request, None)
            .expect("parent should evaluate")
        else {
            panic!("parent should be allowed")
        };

        let child = write_request("/worktree/src/main.ts");
        engine
            .record_decision(&child, PermissionDecision::AllowSession)
            .expect("child grant should record");
        assert!(matches!(
            engine
                .evaluate(&child, Some(&parent))
                .expect("child should evaluate"),
            PermissionOutcome::Allowed(_)
        ));

        let outside = write_request("/other-repository");
        engine
            .record_decision(&outside, PermissionDecision::AllowSession)
            .expect("outside grant should record");
        assert_eq!(
            engine
                .evaluate(&outside, Some(&parent))
                .expect("outside should deny"),
            PermissionOutcome::Denied
        );

        let lexical_escape = write_request("/worktree/../other-repository");
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
                cwd: PathBuf::from("/worktree"),
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
