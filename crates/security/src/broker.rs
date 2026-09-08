//! The permission broker: the single gate every side effect passes through.
//!
//! Three properties do the work here.
//!
//! * **A ceiling that is not a request.** An agent constructed for review is
//!   given a [`CapabilityCeiling`] that forbids writing. A write from that agent
//!   is refused outright and never reaches a human, because a prompt a person
//!   can say yes to is not a guarantee.
//! * **Blocked means stopped.** An undecided request leaves a `pending` row and
//!   an unresolved [`PermissionTicket`]. The agent waits on that ticket; it does
//!   not proceed, retry, or route around it.
//! * **A grant is a durable object, not a flag.** It records who granted it,
//!   when, over what scope, until when, and whether it has since been revoked.
//!   Revoking is a write, not a delete, so the transcript keeps its history.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use mars_activity::{ActivityEntry, ActivityKind, ActivityRecord, Actor};
use mars_storage::Storage;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::classify::{CommandAnalysis, CommandClass, classify_command};
use crate::permissions::{Capability, PermissionError, PermissionScope};

/// How long a grant lives before it has to be asked for again.
const ALLOW_ONCE_TTL: Duration = Duration::from_secs(10 * 60);
const ALLOW_TASK_TTL: Duration = Duration::from_secs(12 * 60 * 60);
const ALLOW_PATTERN_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// The highest command class an agent is permitted to even ask about.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandCeiling {
    None,
    ReadOnly,
    Mutating,
    Destructive,
}

impl CommandCeiling {
    const fn severity(self) -> u8 {
        match self {
            Self::None => 0,
            Self::ReadOnly => 1,
            Self::Mutating => 3,
            Self::Destructive => 4,
        }
    }

    fn admits(self, class: CommandClass) -> bool {
        // Network is its own axis, gated by `CapabilityCeiling::network`.
        if class == CommandClass::Network {
            return self != Self::None;
        }
        self.severity() > class.severity()
    }
}

/// What an agent may ask for at all. Constructed from a role, never edited by a
/// running agent, and checked before any human is consulted.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityCeiling {
    pub read_files: bool,
    pub write_files: bool,
    pub commands: CommandCeiling,
    pub network: bool,
    pub git_push: bool,
}

impl CapabilityCeiling {
    /// An agent doing the work: it may ask for anything, and a human answers.
    #[must_use]
    pub const fn implementer() -> Self {
        Self {
            read_files: true,
            write_files: true,
            commands: CommandCeiling::Destructive,
            network: true,
            git_push: true,
        }
    }

    /// An agent reviewing another's work. Read-only by construction: there is no
    /// decision a human can make that lets this agent write, run a mutating
    /// command, or push.
    #[must_use]
    pub const fn reviewer() -> Self {
        Self {
            read_files: true,
            write_files: false,
            commands: CommandCeiling::ReadOnly,
            network: false,
            git_push: false,
        }
    }

    /// Why this effect is refused outright, or `None` if it may be asked about.
    fn refusal(&self, effect: &SideEffect, analysis: Option<&CommandAnalysis>) -> Option<String> {
        match effect {
            SideEffect::FileWrite { .. } if !self.write_files => {
                Some("this agent was started read-only and cannot write files".into())
            }
            SideEffect::Network { .. } if !self.network => {
                Some("this agent was started without network access".into())
            }
            SideEffect::GitPush { .. } if !self.git_push => {
                Some("this agent was started without permission to push".into())
            }
            SideEffect::Shell { .. } => {
                let class = analysis.map_or(CommandClass::Mutating, |analysis| analysis.class);
                if class == CommandClass::Network && !self.network {
                    return Some("this agent was started without network access".into());
                }
                if self.commands.admits(class) {
                    None
                } else {
                    Some(format!(
                        "this agent was started with a {} command ceiling and cannot run a {} command",
                        match self.commands {
                            CommandCeiling::None => "no-commands",
                            CommandCeiling::ReadOnly => "read-only",
                            CommandCeiling::Mutating => "mutating",
                            CommandCeiling::Destructive => "destructive",
                        },
                        class.as_str().replace('_', "-")
                    ))
                }
            }
            _ => None,
        }
    }
}

/// Something an agent wants to do that leaves the process.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum SideEffect {
    Shell {
        command: String,
        cwd: PathBuf,
    },
    /// A write to a path outside the task's declared file scope.
    FileWrite {
        path: PathBuf,
        declared_scope: Vec<String>,
    },
    Network {
        host: String,
        detail: String,
    },
    GitPush {
        remote: String,
        branch: String,
        cwd: PathBuf,
    },
}

impl SideEffect {
    fn kind(&self) -> &'static str {
        match self {
            Self::Shell { .. } => "shell",
            Self::FileWrite { .. } => "file_write",
            Self::Network { .. } => "network",
            Self::GitPush { .. } => "git_push",
        }
    }

    fn capability(&self) -> Capability {
        match self {
            Self::Shell { .. } => Capability::RunCommand,
            Self::FileWrite { .. } => Capability::WriteFiles,
            Self::Network { .. } => Capability::Network,
            Self::GitPush { .. } => Capability::MutateGit,
        }
    }

    fn scope(&self, task_id: &str) -> PermissionScope {
        match self {
            Self::Shell { command, cwd } => PermissionScope::Command {
                program: classify_command(command).program,
                cwd: cwd.clone(),
            },
            Self::FileWrite { path, .. } => PermissionScope::Worktree {
                task_id: task_id.to_owned(),
                root: path.clone(),
            },
            Self::Network { host, .. } => PermissionScope::NetworkHost { host: host.clone() },
            Self::GitPush { remote, branch, .. } => PermissionScope::Remote {
                remote: remote.clone(),
                branch: branch.clone(),
            },
        }
    }

    /// The text a pattern grant is matched against.
    fn pattern_subject(&self) -> String {
        match self {
            Self::Shell { command, .. } => command.clone(),
            Self::FileWrite { path, .. } => path.to_string_lossy().replace('\\', "/"),
            Self::Network { host, .. } => host.clone(),
            Self::GitPush { remote, branch, .. } => format!("{remote} {branch}"),
        }
    }

    fn summary(&self) -> String {
        match self {
            Self::Shell { command, cwd } => {
                format!("run `{command}` in {}", cwd.display())
            }
            Self::FileWrite { path, .. } => {
                format!("write {}, which is outside the declared scope", path.display())
            }
            Self::Network { host, detail } => format!("reach {host} ({detail})"),
            Self::GitPush {
                remote,
                branch,
                cwd: _,
            } => format!("push {branch} to {remote}"),
        }
    }

    fn suggested_pattern(&self) -> String {
        match self {
            Self::Shell { command, .. } => classify_command(command).suggested_pattern,
            Self::FileWrite { path, .. } => path
                .parent()
                .map(|parent| format!("{}/*", parent.to_string_lossy().replace('\\', "/")))
                .unwrap_or_else(|| "*".into()),
            Self::Network { host, .. } => host.clone(),
            Self::GitPush { remote, .. } => format!("{remote} *"),
        }
    }
}

/// What a human can answer.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrokerDecision {
    AllowOnce,
    AllowTask,
    AllowPattern,
    Deny,
}

impl BrokerDecision {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AllowOnce => "allow_once",
            Self::AllowTask => "allow_task",
            Self::AllowPattern => "allow_pattern",
            Self::Deny => "deny",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "allow_once" => Some(Self::AllowOnce),
            "allow_task" => Some(Self::AllowTask),
            "allow_pattern" => Some(Self::AllowPattern),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }

    fn ttl(self) -> Option<Duration> {
        match self {
            Self::AllowOnce => Some(ALLOW_ONCE_TTL),
            Self::AllowTask => Some(ALLOW_TASK_TTL),
            Self::AllowPattern => Some(ALLOW_PATTERN_TTL),
            Self::Deny => None,
        }
    }
}

/// What the broker says when asked.
#[derive(Clone, Debug, PartialEq)]
pub enum BrokerOutcome {
    /// A live grant already covers this, or a human has just allowed it.
    Allowed { grant_id: String, reason: String },
    /// Refused. `by_construction` distinguishes a ceiling refusal, which no
    /// human can override, from a person saying no.
    Denied {
        reason: String,
        by_construction: bool,
    },
    /// Waiting for a human. The agent is stopped until [`PermissionTicket`]
    /// resolves.
    Pending { request_id: String },
}

/// A pending request as the conversation shows it.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingRequest {
    pub id: String,
    pub task_id: String,
    pub run_id: Option<String>,
    pub effect: SideEffect,
    pub classification: CommandClass,
    pub summary: String,
    pub reason: String,
    pub suggested_pattern: String,
    pub requested_at: String,
}

/// A durable grant, as Settings lists it.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantView {
    pub id: String,
    pub task_id: Option<String>,
    pub capability: String,
    pub scope: PermissionScope,
    pub pattern: Option<String>,
    pub decision: String,
    pub uses_remaining: Option<i64>,
    pub granted_by: String,
    pub granted_at: String,
    pub expires_at: Option<String>,
    pub revoked_at: Option<String>,
    pub revoked_by: Option<String>,
    /// Live now: not revoked and not expired.
    pub active: bool,
}

/// The handle a blocked agent waits on.
pub struct PermissionTicket {
    request_id: String,
    receiver: oneshot::Receiver<BrokerOutcome>,
}

impl PermissionTicket {
    #[must_use]
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// Wait until a human decides. Returns `None` only if the broker is dropped
    /// before a decision is made, which the caller must treat as a denial.
    pub async fn resolve(self) -> Option<BrokerOutcome> {
        self.receiver.await.ok()
    }
}

#[derive(Default)]
struct WaitRegistry {
    waiters: HashMap<String, Vec<oneshot::Sender<BrokerOutcome>>>,
}

/// The broker. One per Mars process; grants and requests live in SQLite so a
/// restart keeps them.
#[derive(Clone)]
pub struct PermissionBroker {
    storage: Arc<Storage>,
    activity: Option<ActivityRecord>,
    waiters: Arc<Mutex<WaitRegistry>>,
}

impl PermissionBroker {
    #[must_use]
    pub fn new(storage: Arc<Storage>) -> Self {
        Self {
            storage,
            activity: None,
            waiters: Arc::new(Mutex::new(WaitRegistry::default())),
        }
    }

    /// Mirror every request and decision into the task's activity record, so the
    /// transcript says who decided what and when.
    #[must_use]
    pub fn recording_to(mut self, activity: ActivityRecord) -> Self {
        self.activity = Some(activity);
        self
    }

    /// Ask for permission. Returns immediately when a live grant covers the
    /// effect or the ceiling refuses it; otherwise it records a pending request
    /// and hands back a ticket to wait on.
    pub fn request(
        &self,
        request: &BrokerRequest,
    ) -> Result<(BrokerOutcome, Option<PermissionTicket>), PermissionError> {
        let analysis = match &request.effect {
            SideEffect::Shell { command, .. } => Some(classify_command(command)),
            _ => None,
        };
        let classification = classification_of(&request.effect, analysis.as_ref());

        if let Some(reason) = request.ceiling.refusal(&request.effect, analysis.as_ref()) {
            let outcome = BrokerOutcome::Denied {
                reason: reason.clone(),
                by_construction: true,
            };
            let request_id = self.persist_request(request, classification, &reason)?;
            self.finalize(
                &request_id,
                request,
                BrokerDecision::Deny,
                "mars",
                Some(&reason),
                None,
            )?;
            return Ok((outcome, None));
        }

        if let Some(grant) = self.matching_grant(request)? {
            self.consume_use(&grant)?;
            return Ok((
                BrokerOutcome::Allowed {
                    grant_id: grant.id,
                    reason: format!("covered by an existing {} grant", grant.decision),
                },
                None,
            ));
        }

        let reason = analysis.as_ref().map_or_else(
            || describe_effect(&request.effect),
            |analysis| analysis.reason.clone(),
        );
        let request_id = self.persist_request(request, classification, &reason)?;
        self.record_activity(
            request,
            ActivityKind::PermissionRequest,
            Actor::Agent,
            &request.agent_label,
            &request.effect.summary(),
            json!({
                "requestId": request_id,
                "classification": classification.as_str(),
                "reason": reason,
                "effect": request.effect,
                "suggestedPattern": request.effect.suggested_pattern(),
            }),
        )?;

        let (sender, receiver) = oneshot::channel();
        self.waiters()?
            .waiters
            .entry(request_id.clone())
            .or_default()
            .push(sender);

        Ok((
            BrokerOutcome::Pending {
                request_id: request_id.clone(),
            },
            Some(PermissionTicket {
                request_id,
                receiver,
            }),
        ))
    }

    /// Record a human's answer and release whoever is waiting on it.
    pub fn decide(
        &self,
        request_id: &str,
        decision: BrokerDecision,
        decided_by: &str,
    ) -> Result<BrokerOutcome, PermissionError> {
        let stored = self
            .stored_request(request_id)?
            .ok_or_else(|| PermissionError::RequestNotFound(request_id.to_owned()))?;
        if stored.state != "pending" {
            return Err(PermissionError::RequestAlreadyDecided(request_id.to_owned()));
        }

        let outcome = self.finalize(
            request_id,
            &stored.request,
            decision,
            decided_by,
            None,
            Some(stored.effect_pattern.clone()),
        )?;

        if let Some(senders) = self.waiters()?.waiters.remove(request_id) {
            for sender in senders {
                let _ = sender.send(outcome.clone());
            }
        }
        Ok(outcome)
    }

    /// Every request still waiting on a person, oldest first.
    pub fn pending(&self, task_id: &str) -> Result<Vec<PendingRequest>, PermissionError> {
        let rows = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, task_id, run_id, effect_json, classification, summary, requested_at
                 FROM permission_requests
                 WHERE task_id = ?1 AND state = 'pending'
                 ORDER BY requested_at, id",
            )?;
            let mapped = statement.query_map([task_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                ))
            })?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()
        })?;

        rows.into_iter()
            .map(
                |(id, task_id, run_id, effect_json, classification, summary, requested_at)| {
                    let effect: SideEffect = serde_json::from_str(&effect_json)?;
                    let reason = match &effect {
                        SideEffect::Shell { command, .. } => classify_command(command).reason,
                        other => describe_effect(other),
                    };
                    Ok(PendingRequest {
                        id,
                        task_id,
                        run_id,
                        classification: CommandClass::parse(&classification)
                            .unwrap_or(CommandClass::Mutating),
                        summary,
                        reason,
                        suggested_pattern: effect.suggested_pattern(),
                        effect,
                        requested_at,
                    })
                },
            )
            .collect()
    }

    /// Grants as Settings shows them: every grant ever made, with whether it is
    /// still live.
    pub fn grants(&self, task_id: Option<&str>) -> Result<Vec<GrantView>, PermissionError> {
        let now = read_now(&self.storage)?;
        let rows = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, task_id, capability, scope_json, pattern, decision, uses_remaining,
                        granted_by, granted_at, expires_at, revoked_at, revoked_by
                 FROM permission_grants
                 WHERE (?1 IS NULL OR task_id = ?1)
                 ORDER BY granted_at DESC, id",
            )?;
            let mapped = statement.query_map([task_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                ))
            })?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()
        })?;

        rows.into_iter()
            .map(|row| {
                let (
                    id,
                    task_id,
                    capability,
                    scope_json,
                    pattern,
                    decision,
                    uses_remaining,
                    granted_by,
                    granted_at,
                    expires_at,
                    revoked_at,
                    revoked_by,
                ) = row;
                let active = revoked_at.is_none()
                    && expires_at
                        .as_deref()
                        .is_none_or(|expiry| expiry > now.as_str())
                    && uses_remaining.is_none_or(|remaining| remaining > 0);
                Ok(GrantView {
                    id,
                    task_id,
                    capability,
                    scope: serde_json::from_str(&scope_json)?,
                    pattern,
                    decision,
                    uses_remaining,
                    granted_by,
                    granted_at,
                    expires_at,
                    revoked_at,
                    revoked_by,
                    active,
                })
            })
            .collect()
    }

    /// Revoke a grant. The row stays; only its revocation is written.
    pub fn revoke(&self, grant_id: &str, revoked_by: &str) -> Result<(), PermissionError> {
        let now = read_now(&self.storage)?;
        let changed = self.storage.with_connection(|connection| {
            connection.execute(
                "UPDATE permission_grants
                 SET revoked_at = ?2, revoked_by = ?3
                 WHERE id = ?1 AND revoked_at IS NULL",
                params![grant_id, &now, revoked_by],
            )
        })?;
        if changed == 0 {
            return Err(PermissionError::GrantNotFound(grant_id.to_owned()));
        }
        Ok(())
    }

    fn finalize(
        &self,
        request_id: &str,
        request: &BrokerRequest,
        decision: BrokerDecision,
        decided_by: &str,
        refusal: Option<&str>,
        pattern: Option<String>,
    ) -> Result<BrokerOutcome, PermissionError> {
        let now = read_now(&self.storage)?;
        let grant_id = if decision == BrokerDecision::Deny {
            None
        } else {
            Some(self.create_grant(request_id, request, decision, decided_by, pattern, &now)?)
        };

        self.storage.with_connection(|connection| {
            connection.execute(
                "UPDATE permission_requests
                 SET state = 'decided', decision = ?2, decided_by = ?3, decided_at = ?4,
                     grant_id = ?5
                 WHERE id = ?1",
                params![
                    request_id,
                    decision.as_str(),
                    decided_by,
                    &now,
                    grant_id.as_deref(),
                ],
            )?;
            Ok(())
        })?;

        let actor = if refusal.is_some() {
            Actor::System
        } else {
            Actor::Human
        };
        self.record_activity(
            request,
            ActivityKind::PermissionDecision,
            actor,
            decided_by,
            &match refusal {
                Some(reason) => format!("Refused: {reason}"),
                None => format!("{} — {}", decision.as_str(), request.effect.summary()),
            },
            json!({
                "requestId": request_id,
                "decision": decision.as_str(),
                "decidedBy": decided_by,
                "grantId": grant_id,
                "byConstruction": refusal.is_some(),
            }),
        )?;

        Ok(match (decision, grant_id) {
            (BrokerDecision::Deny, _) => BrokerOutcome::Denied {
                reason: refusal.map_or_else(
                    || format!("{decided_by} denied this request"),
                    ToOwned::to_owned,
                ),
                by_construction: refusal.is_some(),
            },
            (_, Some(grant_id)) => BrokerOutcome::Allowed {
                grant_id,
                reason: format!("{decided_by} chose {}", decision.as_str()),
            },
            (_, None) => BrokerOutcome::Denied {
                reason: "the decision produced no grant".into(),
                by_construction: false,
            },
        })
    }

    fn create_grant(
        &self,
        request_id: &str,
        request: &BrokerRequest,
        decision: BrokerDecision,
        granted_by: &str,
        pattern: Option<String>,
        now: &str,
    ) -> Result<String, PermissionError> {
        let id = Uuid::now_v7().to_string();
        let scope = request.effect.scope(&request.task_id);
        let scope_json = serde_json::to_string(&scope)?;
        let pattern = match decision {
            BrokerDecision::AllowPattern => {
                Some(pattern.unwrap_or_else(|| request.effect.suggested_pattern()))
            }
            _ => None,
        };
        let uses_remaining = match decision {
            BrokerDecision::AllowOnce => Some(1_i64),
            _ => None,
        };
        let task_scope = match decision {
            BrokerDecision::AllowPattern => None,
            _ => Some(request.task_id.clone()),
        };
        let expires_at = decision
            .ttl()
            .map(|ttl| read_expiry(&self.storage, ttl))
            .transpose()?;

        self.storage.with_connection(|connection| {
            connection.execute(
                "INSERT INTO permission_grants
                     (id, task_id, capability, scope_json, pattern, decision, uses_remaining,
                      granted_by, granted_at, expires_at, revoked_at, revoked_by,
                      source_request_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, NULL, ?11)",
                params![
                    &id,
                    task_scope.as_deref(),
                    capability_name(request.effect.capability()),
                    &scope_json,
                    pattern.as_deref(),
                    decision.as_str(),
                    uses_remaining,
                    granted_by,
                    now,
                    expires_at.as_deref(),
                    request_id,
                ],
            )?;
            Ok(())
        })?;
        Ok(id)
    }

    fn matching_grant(
        &self,
        request: &BrokerRequest,
    ) -> Result<Option<MatchedGrant>, PermissionError> {
        let now = read_now(&self.storage)?;
        let capability = capability_name(request.effect.capability());
        let scope = request.effect.scope(&request.task_id);
        let subject = request.effect.pattern_subject();

        let rows = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, scope_json, pattern, decision, uses_remaining
                 FROM permission_grants
                 WHERE capability = ?1
                   AND revoked_at IS NULL
                   AND (expires_at IS NULL OR expires_at > ?2)
                   AND (task_id IS NULL OR task_id = ?3)
                   AND (uses_remaining IS NULL OR uses_remaining > 0)
                 ORDER BY granted_at DESC, id",
            )?;
            let mapped = statement.query_map(params![capability, &now, &request.task_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                ))
            })?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()
        })?;

        for (id, scope_json, pattern, decision, uses_remaining) in rows {
            let granted_scope: PermissionScope = serde_json::from_str(&scope_json)?;
            let matches = match pattern.as_deref() {
                Some(pattern) => glob_match(pattern, &subject),
                None => granted_scope.contains(&scope),
            };
            if matches {
                return Ok(Some(MatchedGrant {
                    id,
                    decision,
                    uses_remaining,
                }));
            }
        }
        Ok(None)
    }

    fn consume_use(&self, grant: &MatchedGrant) -> Result<(), PermissionError> {
        if grant.uses_remaining.is_none() {
            return Ok(());
        }
        self.storage.with_connection(|connection| {
            connection.execute(
                "UPDATE permission_grants
                 SET uses_remaining = MAX(uses_remaining - 1, 0)
                 WHERE id = ?1 AND uses_remaining IS NOT NULL",
                [&grant.id],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    fn persist_request(
        &self,
        request: &BrokerRequest,
        classification: CommandClass,
        _reason: &str,
    ) -> Result<String, PermissionError> {
        let id = Uuid::now_v7().to_string();
        let scope_json = serde_json::to_string(&request.effect.scope(&request.task_id))?;
        let effect_json = serde_json::to_string(&request.effect)?;
        let now = read_now(&self.storage)?;
        self.storage.with_connection(|connection| {
            connection.execute(
                "INSERT INTO permission_requests
                     (id, task_id, run_id, effect_kind, classification, capability, scope_json,
                      effect_json, summary, state, decision, decided_by, decided_at, grant_id,
                      requested_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'pending', NULL, NULL, NULL, NULL,
                         ?10)",
                params![
                    &id,
                    &request.task_id,
                    request.run_id.as_deref(),
                    request.effect.kind(),
                    classification.as_str(),
                    capability_name(request.effect.capability()),
                    &scope_json,
                    &effect_json,
                    request.effect.summary(),
                    &now,
                ],
            )?;
            Ok(())
        })?;
        Ok(id)
    }

    fn stored_request(&self, request_id: &str) -> Result<Option<StoredRequest>, PermissionError> {
        let row = self.storage.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT task_id, run_id, effect_json, state
                     FROM permission_requests WHERE id = ?1",
                    [request_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    },
                )
                .optional()
        })?;
        let Some((task_id, run_id, effect_json, state)) = row else {
            return Ok(None);
        };
        let effect: SideEffect = serde_json::from_str(&effect_json)?;
        let effect_pattern = effect.suggested_pattern();
        Ok(Some(StoredRequest {
            state,
            effect_pattern,
            request: BrokerRequest {
                task_id,
                run_id,
                agent_label: "agent".into(),
                // A decision never re-checks the ceiling; the ceiling already
                // ran when the request was made, and a pending request only
                // exists because it passed.
                ceiling: CapabilityCeiling::implementer(),
                effect,
            },
        }))
    }

    fn record_activity(
        &self,
        request: &BrokerRequest,
        kind: ActivityKind,
        actor: Actor,
        actor_label: &str,
        summary: &str,
        payload: serde_json::Value,
    ) -> Result<(), PermissionError> {
        let Some(activity) = self.activity.as_ref() else {
            return Ok(());
        };
        let mut entry = ActivityEntry::new(&request.task_id, kind, actor, actor_label, summary)
            .with_payload(payload);
        if let Some(run_id) = request.run_id.as_deref() {
            entry = entry.in_run(run_id);
        }
        activity
            .append(entry)
            .map_err(|error| PermissionError::Activity(error.to_string()))?;
        Ok(())
    }

    fn waiters(&self) -> Result<MutexGuard<'_, WaitRegistry>, PermissionError> {
        self.waiters
            .lock()
            .map_err(|_| PermissionError::StatePoisoned)
    }
}

/// One agent's ask.
#[derive(Clone, Debug)]
pub struct BrokerRequest {
    pub task_id: String,
    pub run_id: Option<String>,
    pub agent_label: String,
    pub ceiling: CapabilityCeiling,
    pub effect: SideEffect,
}

struct StoredRequest {
    state: String,
    effect_pattern: String,
    request: BrokerRequest,
}

struct MatchedGrant {
    id: String,
    decision: String,
    uses_remaining: Option<i64>,
}

fn classification_of(effect: &SideEffect, analysis: Option<&CommandAnalysis>) -> CommandClass {
    match effect {
        SideEffect::Shell { .. } => analysis.map_or(CommandClass::Mutating, |value| value.class),
        SideEffect::FileWrite { .. } => CommandClass::Mutating,
        SideEffect::Network { .. } => CommandClass::Network,
        SideEffect::GitPush { .. } => CommandClass::Destructive,
    }
}

fn describe_effect(effect: &SideEffect) -> String {
    match effect {
        SideEffect::Shell { command, .. } => format!("`{command}` was not recognised as read-only"),
        SideEffect::FileWrite { declared_scope, .. } => format!(
            "the task declared its file scope as {}",
            if declared_scope.is_empty() {
                "empty".to_owned()
            } else {
                declared_scope.join(", ")
            }
        ),
        SideEffect::Network { host, .. } => format!("{host} is outside the worktree"),
        SideEffect::GitPush { remote, .. } => {
            format!("pushing to {remote} publishes work outside this machine")
        }
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

/// Every timestamp the broker writes or compares comes from SQLite, so grants,
/// requests, and expiry checks all share one clock and one lexicographically
/// comparable format.
fn read_now(storage: &Storage) -> Result<String, PermissionError> {
    Ok(storage.with_connection(|connection| {
        connection.query_row(
            "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            [],
            |row| row.get(0),
        )
    })?)
}

fn read_expiry(storage: &Storage, ttl: Duration) -> Result<String, PermissionError> {
    let modifier = format!("+{} seconds", ttl.as_secs());
    Ok(storage.with_connection(|connection| {
        connection.query_row(
            "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?1)",
            [&modifier],
            |row| row.get(0),
        )
    })?)
}

/// A deliberately small glob: `*` spans anything, `?` is one character. Grants
/// are read by humans, so the pattern language has to be one they can predict.
#[must_use]
pub fn glob_match(pattern: &str, subject: &str) -> bool {
    fn matches(pattern: &[char], subject: &[char]) -> bool {
        match pattern.first() {
            None => subject.is_empty(),
            Some('*') => {
                if matches(&pattern[1..], subject) {
                    return true;
                }
                !subject.is_empty() && matches(pattern, &subject[1..])
            }
            Some('?') => !subject.is_empty() && matches(&pattern[1..], &subject[1..]),
            Some(expected) => {
                !subject.is_empty()
                    && subject[0].eq_ignore_ascii_case(expected)
                    && matches(&pattern[1..], &subject[1..])
            }
        }
    }
    let pattern: Vec<char> = pattern.trim().chars().collect();
    let subject: Vec<char> = subject.trim().chars().collect();
    matches(&pattern, &subject)
}
