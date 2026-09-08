//! The context pack as an inspectable object.
//!
//! [`ContextPack`] answers "what was retrieved". A [`RunContextPack`] answers
//! the question the UI actually asks before a run: *exactly what will this agent
//! see, and what will it be allowed to do?* It adds the two things retrieval
//! cannot know — the tools the agent is given and the permissions it holds — and
//! it carries the prompt verbatim rather than a summary of it.
//!
//! The digest is over that prompt. Because the agent is launched with the same
//! string, a pack that was previewed and a pack that was sent can be compared
//! byte for byte (ADR-021), and the run records which one it used.

use std::sync::Arc;

use mars_security::{CapabilityCeiling, GrantView};
use mars_storage::Storage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::context_pack::{
    ContextPack, ContextPackError, ContextPackRequest, ContextPackService, PackExclusion,
    PackSection, SectionUsage, render_context_pack,
};

/// One tool the agent will be given, and whether it can change anything.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolGrant {
    pub name: String,
    pub description: String,
    pub read_only: bool,
}

impl ToolGrant {
    #[must_use]
    pub fn read_only(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            read_only: true,
        }
    }

    #[must_use]
    pub fn mutating(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            read_only: false,
        }
    }
}

/// A standing grant the agent will not be asked about again.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StandingGrant {
    pub capability: String,
    pub pattern: Option<String>,
    pub decision: String,
    pub granted_by: String,
    pub expires_at: Option<String>,
}

/// What the agent may do, as the approval dialog states it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionsView {
    pub role: String,
    pub ceiling: CapabilityCeiling,
    /// The file globs the approved worktree plan declared.
    pub declared_scope: Vec<String>,
    pub standing_grants: Vec<StandingGrant>,
    /// Always true: anything not covered above stops the agent and asks.
    pub everything_else_asks: bool,
}

/// One included item, without its body — the index a person scans.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackEntry {
    pub section: PackSection,
    pub kind: String,
    pub entity_ref: String,
    pub title: String,
    pub provenance: String,
    pub freshness: String,
    pub source_location: String,
    pub reason: String,
    pub byte_size: usize,
}

/// Everything an agent will see, assembled and inspectable.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunContextPack {
    pub task_id: String,
    pub agent_id: String,
    pub role: String,
    pub adapter: String,
    pub entries: Vec<PackEntry>,
    pub section_usage: Vec<SectionUsage>,
    pub exclusions: Vec<PackExclusion>,
    pub tools: Vec<ToolGrant>,
    pub permissions: PermissionsView,
    /// The exact string the agent is launched with.
    pub prompt: String,
    pub total_bytes: usize,
    /// SHA-256 of `prompt`. What a run records, and what launch revalidates.
    pub digest: String,
}

impl RunContextPack {
    /// Entity references of one kind, in pack order — `file`, `symbol`,
    /// `memory`, `skill`, `context`, `task`.
    #[must_use]
    pub fn of_kind(&self, kind: &str) -> Vec<&str> {
        self.entries
            .iter()
            .filter(|entry| entry.kind == kind)
            .map(|entry| entry.entity_ref.as_str())
            .collect()
    }
}

/// What to assemble.
#[derive(Clone, Debug)]
pub struct RunPackRequest {
    pub task_id: String,
    pub agent_id: String,
    pub adapter: String,
    pub role: String,
    pub ceiling: CapabilityCeiling,
    pub declared_scope: Vec<String>,
    pub tools: Vec<ToolGrant>,
    pub assignment: String,
    pub max_bytes: usize,
    pub excluded_entity_refs: Vec<String>,
}

pub struct RunContextPackBuilder {
    packs: ContextPackService,
    grants: Vec<GrantView>,
}

impl RunContextPackBuilder {
    #[must_use]
    pub fn new(storage: Arc<Storage>) -> Self {
        Self {
            packs: ContextPackService::new(storage),
            grants: Vec::new(),
        }
    }

    /// Show the grants that already stand for this task, so a reader can see
    /// what the agent will *not* be stopped for.
    #[must_use]
    pub fn with_grants(mut self, grants: Vec<GrantView>) -> Self {
        self.grants = grants;
        self
    }

    pub fn build(&self, request: RunPackRequest) -> Result<RunContextPack, ContextPackError> {
        let retrieved = self.packs.build(ContextPackRequest {
            task_id: request.task_id.clone(),
            adapter: request.adapter.clone(),
            max_bytes: request.max_bytes,
            excluded_entity_refs: request.excluded_entity_refs.clone(),
        })?;

        // A reviewer's tool list is filtered by the same ceiling that will
        // refuse the effect later, so the preview cannot promise a tool the
        // broker would deny.
        let tools = request
            .tools
            .iter()
            .filter(|tool| tool.read_only || request.ceiling.write_files)
            .cloned()
            .collect::<Vec<_>>();

        let permissions = PermissionsView {
            role: request.role.clone(),
            ceiling: request.ceiling,
            declared_scope: request.declared_scope.clone(),
            standing_grants: self
                .grants
                .iter()
                .filter(|grant| grant.active)
                .map(|grant| StandingGrant {
                    capability: grant.capability.clone(),
                    pattern: grant.pattern.clone(),
                    decision: grant.decision.clone(),
                    granted_by: grant.granted_by.clone(),
                    expires_at: grant.expires_at.clone(),
                })
                .collect(),
            everything_else_asks: true,
        };

        let prompt = render_prompt(&retrieved, &tools, &permissions, &request.assignment);
        let digest = digest_of(&prompt);

        Ok(RunContextPack {
            task_id: request.task_id,
            agent_id: request.agent_id,
            role: request.role,
            adapter: request.adapter,
            entries: retrieved
                .items
                .iter()
                .map(|item| PackEntry {
                    section: item.section,
                    kind: kind_of(&item.entity_ref),
                    entity_ref: item.entity_ref.clone(),
                    title: item.title.clone(),
                    provenance: item.provenance.clone(),
                    freshness: item.freshness.clone(),
                    source_location: item.source_location.clone(),
                    reason: item.reason.clone(),
                    byte_size: item.byte_size,
                })
                .collect(),
            section_usage: retrieved.section_usage.clone(),
            exclusions: retrieved.exclusions.clone(),
            tools,
            permissions,
            total_bytes: prompt.len(),
            prompt,
            digest,
        })
    }

    /// Rebuild the pack and confirm it is still the one that was approved.
    /// A pack whose digest has moved must be looked at again before it is sent.
    pub fn revalidate(
        &self,
        request: RunPackRequest,
        approved_digest: &str,
    ) -> Result<RunContextPack, ContextPackError> {
        let rebuilt = self.build(request)?;
        if rebuilt.digest == approved_digest {
            Ok(rebuilt)
        } else {
            Err(ContextPackError::PackChanged {
                approved: approved_digest.to_owned(),
                current: rebuilt.digest,
            })
        }
    }
}

/// The prompt, permissions and all. The agent is told what it may do in the
/// same text a person read, so the two cannot disagree.
fn render_prompt(
    pack: &ContextPack,
    tools: &[ToolGrant],
    permissions: &PermissionsView,
    assignment: &str,
) -> String {
    let mut rendered = render_context_pack(pack, assignment);
    rendered.push_str("\n\n## Tools\n");
    if tools.is_empty() {
        rendered.push_str("No tools are provided.\n");
    } else {
        for tool in tools {
            rendered.push_str(&format!(
                "- {} ({}): {}\n",
                tool.name,
                if tool.read_only {
                    "read-only"
                } else {
                    "may change files"
                },
                tool.description
            ));
        }
    }

    rendered.push_str("\n## Permissions\n");
    rendered.push_str(&format!("Role: {}\n", permissions.role));
    rendered.push_str(&format!(
        "You may {}write files, {}reach the network, and {}push.\n",
        negate(permissions.ceiling.write_files),
        negate(permissions.ceiling.network),
        negate(permissions.ceiling.git_push),
    ));
    if permissions.declared_scope.is_empty() {
        rendered.push_str("This task declared no file scope.\n");
    } else {
        rendered.push_str(&format!(
            "Declared file scope: {}.\n",
            permissions.declared_scope.join(", ")
        ));
    }
    for grant in &permissions.standing_grants {
        rendered.push_str(&format!(
            "- already allowed: {} {} (granted by {})\n",
            grant.capability,
            grant.pattern.as_deref().unwrap_or("in this task"),
            grant.granted_by
        ));
    }
    rendered.push_str("Anything else stops you and asks a person. Do not work around a refusal.\n");
    rendered
}

const fn negate(allowed: bool) -> &'static str {
    if allowed { "" } else { "not " }
}

/// The prefix of an entity reference names what it is.
fn kind_of(entity_ref: &str) -> String {
    entity_ref
        .split_once(':')
        .map_or_else(|| "unknown".to_owned(), |(kind, _)| kind.to_owned())
}

fn digest_of(prompt: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prompt.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
