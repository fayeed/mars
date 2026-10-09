use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use mars_storage::{Storage, StorageError};
use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::policy::AdapterPolicy;
use crate::registry::{RegistryClient, SkillCandidate, SkillFile, SkillSnapshot};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillScope {
    Global,
    Project,
    Task,
    Workflow,
}

impl SkillScope {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Project => "project",
            Self::Task => "task",
            Self::Workflow => "workflow",
        }
    }

    fn parse(value: &str) -> Result<Self, SkillError> {
        match value {
            "global" => Ok(Self::Global),
            "project" => Ok(Self::Project),
            "task" => Ok(Self::Task),
            "workflow" => Ok(Self::Workflow),
            other => Err(SkillError::InvalidStoredScope(other.to_owned())),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectedSkill {
    pub id: String,
    pub name: String,
    pub source: String,
    pub slug: String,
    pub installs: u64,
    pub content_hash: String,
    pub registry_hash: String,
    /// Whether the files received hash to what the registry says they are.
    /// False is shown to the person before they trust anything.
    pub registry_verified: bool,
    pub trust_state: String,
    pub skill_markdown: String,
    pub files: Vec<String>,
    pub changed_files: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInstallation {
    pub id: String,
    pub skill_id: String,
    pub name: String,
    pub source: String,
    /// The registry's path segment for this skill, to look for updates with.
    pub slug: String,
    pub scope: SkillScope,
    pub repository_id: Option<String>,
    pub task_id: Option<String>,
    pub install_path: PathBuf,
    pub pinned_hash: String,
    /// The instructions at the pinned version: what an agent is given.
    pub skill_markdown: String,
    /// The one-line description from `SKILL.md`'s front matter.
    pub description: String,
    pub enabled: bool,
    /// The skill was blocked after this was installed; it reaches no agent.
    pub blocked: bool,
    pub allow_claude: bool,
    pub allow_codex: bool,
    pub installed_at: String,
}

/// A newer version than the one an installation pins, found but not installed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillUpdate {
    pub installation_id: String,
    pub pinned_hash: String,
    pub latest_hash: String,
    /// Files that differ between the pinned and the latest version. Empty
    /// means the installation is current.
    pub changed_files: Vec<String>,
}

#[derive(Debug, Error)]
pub enum SkillError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Network(#[from] reqwest::Error),
    #[error("invalid skills registry source `{0}`; expected owner/repository")]
    InvalidSource(String),
    #[error("skill snapshot hash mismatch: expected {expected}, computed {actual}")]
    HashMismatch { expected: String, actual: String },
    #[error("skills registry returned an invalid snapshot hash `{0}`")]
    InvalidRegistryHash(String),
    #[error("skill snapshot does not contain SKILL.md")]
    MissingSkillMarkdown,
    #[error("unsafe skill file path `{0}`")]
    UnsafePath(String),
    #[error("skill `{0}` has not been inspected")]
    NotInspected(String),
    #[error("skill `{0}` is blocked")]
    Blocked(String),
    #[error("skill installation `{0}` does not exist")]
    InstallationNotFound(String),
    #[error("stored skill scope is invalid: {0}")]
    InvalidStoredScope(String),
    #[error("scope `{scope}` requires {field}")]
    MissingScopeTarget { scope: String, field: &'static str },
    #[error("skill filesystem operation failed for `{path}`: {source}")]
    Filesystem {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to encode or decode skill data: {0}")]
    Json(#[from] serde_json::Error),
    #[error("failed to format skill timestamp: {0}")]
    Timestamp(#[from] time::error::Format),
}

#[derive(Clone)]
pub struct SkillService {
    storage: Arc<Storage>,
    registry: Arc<dyn RegistryClient>,
    install_root: PathBuf,
}

impl SkillService {
    #[must_use]
    pub fn new(
        storage: Arc<Storage>,
        registry: Arc<dyn RegistryClient>,
        install_root: impl AsRef<Path>,
    ) -> Self {
        Self {
            storage,
            registry,
            install_root: install_root.as_ref().to_owned(),
        }
    }

    pub fn search(&self, query: &str) -> Result<Vec<SkillCandidate>, SkillError> {
        self.registry.search(query)
    }

    /// Download a skill and hold it for inspection. Nothing is installed and
    /// nothing already installed changes: an installation keeps the version
    /// it pinned until a person installs another.
    pub fn inspect(&self, candidate: &SkillCandidate) -> Result<InspectedSkill, SkillError> {
        let snapshot = self.registry.download(&candidate.source, &candidate.slug)?;
        validate_snapshot(&snapshot)?;
        let actual = snapshot_hash(&snapshot.files);
        if snapshot.hash.len() != 64
            || !snapshot
                .hash
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        {
            return Err(SkillError::InvalidRegistryHash(snapshot.hash));
        }
        let registry_verified = registry_digest(&snapshot.files) == snapshot.hash.to_lowercase();
        let skill_markdown = skill_markdown_of(&snapshot.files)?;
        let previous: Option<(String, String, String)> =
            self.storage.with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT id, trust_state, snapshot_json FROM skills
                         WHERE source = ?1 AND canonical_name = ?2",
                        params![candidate.source, candidate.name],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .optional()
            })?;
        let changed_files = previous
            .as_ref()
            .map(|(_, _, raw)| serde_json::from_str::<SkillSnapshot>(raw))
            .transpose()?
            .map(|old| changed_paths(&old.files, &snapshot.files))
            .unwrap_or_else(|| {
                snapshot
                    .files
                    .iter()
                    .map(|file| file.path.clone())
                    .collect()
            });
        let id = previous
            .as_ref()
            .map(|(id, _, _)| id.clone())
            .unwrap_or_else(|| Uuid::now_v7().to_string());
        let snapshot_json = serde_json::to_string(&snapshot)?;
        // A block outlives any new version: looking at a skill again is not a
        // decision to trust it. Otherwise trust carries over only when the
        // content is exactly what was trusted before.
        let trust_state = match previous.as_ref() {
            Some((_, trust, _)) if trust == "blocked" => "blocked",
            Some((_, trust, raw)) if raw == &snapshot_json => trust.as_str(),
            _ => "inspected",
        };
        let timestamp = now()?;
        self.storage.with_connection(|connection| {
            connection.execute(
                "INSERT INTO skills
                 (id, canonical_name, source, slug, installs, content_hash, registry_hash, trust_state,
                  skill_markdown, snapshot_json, inspected_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)
                 ON CONFLICT(source, canonical_name) DO UPDATE SET
                   slug = excluded.slug, installs = excluded.installs,
                   content_hash = excluded.content_hash, registry_hash = excluded.registry_hash,
                   trust_state = excluded.trust_state,
                   skill_markdown = excluded.skill_markdown, snapshot_json = excluded.snapshot_json,
                   inspected_at = excluded.inspected_at, updated_at = excluded.updated_at",
                params![
                    id,
                    candidate.name,
                    candidate.source,
                    candidate.slug,
                    i64::try_from(candidate.installs).unwrap_or(i64::MAX),
                    actual,
                    snapshot.hash,
                    trust_state,
                    skill_markdown,
                    snapshot_json,
                    timestamp,
                ],
            )?;
            Ok(())
        })?;
        Ok(InspectedSkill {
            id,
            name: candidate.name.clone(),
            source: candidate.source.clone(),
            slug: candidate.slug.clone(),
            installs: candidate.installs,
            content_hash: actual,
            registry_hash: snapshot.hash,
            registry_verified,
            trust_state: trust_state.into(),
            skill_markdown,
            files: snapshot.files.into_iter().map(|file| file.path).collect(),
            changed_files,
        })
    }

    /// Trust the version last inspected and install it at `scope`.
    ///
    /// The files are written once per content hash under `store/` and shared
    /// by every installation of that version, so two tasks that install the
    /// same skill do not hold two copies, and removing one leaves the other.
    #[allow(clippy::too_many_arguments)]
    pub fn trust_and_install(
        &self,
        skill_id: &str,
        repository_id: Option<&str>,
        task_id: Option<&str>,
        scope: SkillScope,
        allow_claude: bool,
        allow_codex: bool,
    ) -> Result<SkillInstallation, SkillError> {
        validate_scope(scope, repository_id, task_id)?;
        // A global skill belongs to no repository, and a project skill to no
        // task: the identity of an installation is its scope and its target.
        let repository_id = (scope != SkillScope::Global)
            .then_some(repository_id)
            .flatten();
        let task_id = matches!(scope, SkillScope::Task | SkillScope::Workflow)
            .then_some(task_id)
            .flatten();
        let Some((name, hash, trust_state, markdown, raw_snapshot)) = self
            .storage
            .with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT canonical_name, content_hash, trust_state, skill_markdown, snapshot_json
                         FROM skills WHERE id = ?1",
                        [skill_id],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?)),
                    )
                    .optional()
            })?
        else {
            return Err(SkillError::NotInspected(skill_id.to_owned()));
        };
        if trust_state == "blocked" {
            return Err(SkillError::Blocked(name));
        }
        let snapshot: SkillSnapshot = serde_json::from_str(&raw_snapshot)?;
        validate_snapshot(&snapshot)?;
        let actual_hash = snapshot_hash(&snapshot.files);
        if actual_hash != hash {
            return Err(SkillError::HashMismatch {
                expected: hash,
                actual: actual_hash,
            });
        }
        let final_path = self.install_root.join("store").join(&hash);
        let created = if final_path.is_dir() {
            false
        } else {
            install_snapshot(&self.install_root, &final_path, &snapshot)?;
            true
        };
        let timestamp = now()?;
        let path_text = final_path.to_string_lossy().into_owned();
        let result = self.storage.with_transaction(|transaction| {
            transaction.execute(
                "UPDATE skills SET trust_state = 'trusted', updated_at = ?2 WHERE id = ?1",
                params![skill_id, timestamp],
            )?;
            let existing: Option<String> = transaction
                .query_row(
                    "SELECT id FROM skill_installations
                     WHERE skill_id = ?1 AND scope = ?2 AND repository_id IS ?3 AND task_id IS ?4",
                    params![skill_id, scope.as_str(), repository_id, task_id],
                    |row| row.get(0),
                )
                .optional()?;
            let stored_id = match existing {
                Some(id) => {
                    transaction.execute(
                        "UPDATE skill_installations
                         SET install_path = ?2, pinned_hash = ?3, skill_markdown = ?4,
                             enabled = 1, updated_at = ?5
                         WHERE id = ?1",
                        params![id, path_text, hash, markdown, timestamp],
                    )?;
                    id
                }
                None => {
                    let id = Uuid::now_v7().to_string();
                    transaction.execute(
                        "INSERT INTO skill_installations
                         (id, skill_id, repository_id, task_id, scope, install_path, pinned_hash,
                          skill_markdown, enabled, installed_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9, ?9)",
                        params![
                            id,
                            skill_id,
                            repository_id,
                            task_id,
                            scope.as_str(),
                            path_text,
                            hash,
                            markdown,
                            timestamp
                        ],
                    )?;
                    id
                }
            };
            for (adapter, allowed) in [
                (AdapterPolicy::Claude, allow_claude),
                (AdapterPolicy::Codex, allow_codex),
            ] {
                set_policy(transaction, &stored_id, adapter, allowed)?;
            }
            Ok(stored_id)
        });
        let stored_id = match result {
            Ok(id) => id,
            Err(error) => {
                if created {
                    let _ = fs::remove_dir_all(&final_path);
                }
                return Err(error.into());
            }
        };
        self.installation(&stored_id)
    }

    /// Stop trusting a skill. Every installation of it stops reaching agents
    /// until a person inspects and installs it again.
    pub fn block(&self, skill_id: &str) -> Result<(), SkillError> {
        self.storage.with_connection(|connection| {
            connection.execute(
                "UPDATE skills SET trust_state = 'blocked' WHERE id = ?1",
                [skill_id],
            )?;
            connection.execute(
                "UPDATE skill_installations SET enabled = 0 WHERE skill_id = ?1",
                [skill_id],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    /// Turn an installation on or off without uninstalling it.
    pub fn set_enabled(
        &self,
        installation_id: &str,
        enabled: bool,
    ) -> Result<SkillInstallation, SkillError> {
        let installation = self.installation(installation_id)?;
        if enabled && installation.blocked {
            return Err(SkillError::Blocked(installation.name));
        }
        self.storage.with_connection(|connection| {
            connection.execute(
                "UPDATE skill_installations SET enabled = ?2, updated_at = ?3 WHERE id = ?1",
                params![installation_id, enabled, now().unwrap_or_default()],
            )?;
            Ok(())
        })?;
        self.installation(installation_id)
    }

    /// Allow or refuse an installation for one agent.
    pub fn set_adapter_allowed(
        &self,
        installation_id: &str,
        adapter: AdapterPolicy,
        allowed: bool,
    ) -> Result<SkillInstallation, SkillError> {
        self.installation(installation_id)?;
        self.storage.with_transaction(|transaction| {
            set_policy(transaction, installation_id, adapter, allowed)
        })?;
        self.installation(installation_id)
    }

    /// Whether the registry has a newer version than the one pinned. Nothing
    /// is written: updating is inspecting and installing the new version.
    pub fn check_update(&self, installation_id: &str) -> Result<SkillUpdate, SkillError> {
        let (source, slug, pinned_hash, install_path): (String, String, String, String) = self
            .storage
            .with_connection(|connection| {
                connection.query_row(
                    "SELECT s.source, s.slug, i.pinned_hash, i.install_path
                     FROM skill_installations i JOIN skills s ON s.id = i.skill_id
                     WHERE i.id = ?1",
                    [installation_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
            })
            .map_err(|_| SkillError::InstallationNotFound(installation_id.to_owned()))?;
        let latest = self.registry.download(&source, &slug)?;
        validate_snapshot(&latest)?;
        let latest_hash = snapshot_hash(&latest.files);
        let pinned_files = read_installed_files(Path::new(&install_path)).unwrap_or_default();
        let changed_files = if latest_hash == pinned_hash {
            Vec::new()
        } else {
            changed_paths(&pinned_files, &latest.files)
        };
        Ok(SkillUpdate {
            installation_id: installation_id.to_owned(),
            pinned_hash,
            latest_hash,
            changed_files,
        })
    }

    /// Installations that reach a task: its own, its repository's, and the
    /// global ones. Disabled and blocked ones are included, marked as such, so
    /// a list can say why a skill is not being used.
    pub fn list_for_task(&self, task_id: &str) -> Result<Vec<SkillInstallation>, SkillError> {
        let repository_id: Option<String> = self.storage.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT repository_id FROM tasks WHERE id = ?1",
                    [task_id],
                    |row| row.get(0),
                )
                .optional()
        })?;
        self.list_where(
            "i.task_id = ?1 OR (i.scope = 'project' AND i.repository_id = ?2) OR i.scope = 'global'",
            params![task_id, repository_id],
        )
    }

    /// Every installation that matters to a repository: global ones, the
    /// repository's own, and those of each of its tasks.
    pub fn list_for_repository(
        &self,
        repository_id: &str,
    ) -> Result<Vec<SkillInstallation>, SkillError> {
        self.list_where(
            "i.scope = 'global' OR i.repository_id = ?1
             OR i.task_id IN (SELECT id FROM tasks WHERE repository_id = ?1)",
            params![repository_id],
        )
    }

    /// The installations an agent of `adapter` working in `task_id` is given:
    /// enabled, not blocked, allowed for that agent, and in scope.
    pub fn applicable(
        &self,
        task_id: &str,
        adapter: AdapterPolicy,
    ) -> Result<Vec<SkillInstallation>, SkillError> {
        Ok(self
            .list_for_task(task_id)?
            .into_iter()
            .filter(|installation| installation.enabled && !installation.blocked)
            .filter(|installation| match adapter {
                AdapterPolicy::Claude => installation.allow_claude,
                AdapterPolicy::Codex => installation.allow_codex,
            })
            .collect())
    }

    pub fn installation(&self, installation_id: &str) -> Result<SkillInstallation, SkillError> {
        self.list_where("i.id = ?1", params![installation_id])?
            .into_iter()
            .next()
            .ok_or_else(|| SkillError::InstallationNotFound(installation_id.to_owned()))
    }

    fn list_where(
        &self,
        predicate: &str,
        parameters: impl rusqlite::Params,
    ) -> Result<Vec<SkillInstallation>, SkillError> {
        struct Row {
            id: String,
            skill_id: String,
            name: String,
            source: String,
            slug: String,
            scope: String,
            repository_id: Option<String>,
            task_id: Option<String>,
            path: String,
            hash: String,
            markdown: String,
            enabled: bool,
            trust: String,
            installed_at: String,
        }
        let rows: Vec<Row> = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT i.id, i.skill_id, s.canonical_name, s.source, i.scope, i.repository_id,
                        i.task_id, i.install_path, i.pinned_hash,
                        CASE WHEN i.skill_markdown = '' THEN s.skill_markdown ELSE i.skill_markdown END,
                        i.enabled, s.trust_state, i.installed_at, s.slug
                 FROM skill_installations i JOIN skills s ON s.id = i.skill_id
                 WHERE {predicate}
                 ORDER BY i.installed_at DESC, i.id DESC"
            ))?;
            statement
                .query_map(parameters, |row| {
                    Ok(Row {
                        id: row.get(0)?,
                        skill_id: row.get(1)?,
                        name: row.get(2)?,
                        source: row.get(3)?,
                        scope: row.get(4)?,
                        repository_id: row.get(5)?,
                        task_id: row.get(6)?,
                        path: row.get(7)?,
                        hash: row.get(8)?,
                        markdown: row.get(9)?,
                        enabled: row.get::<_, i64>(10)? == 1,
                        trust: row.get(11)?,
                        installed_at: row.get(12)?,
                        slug: row.get(13)?,
                    })
                })?
                .collect()
        })?;
        rows.into_iter()
            .map(|row| {
                let policies: Vec<(String, bool)> = self.storage.with_connection(|connection| {
                    let mut statement = connection.prepare(
                        "SELECT adapter_kind, allowed FROM skill_agent_policies WHERE installation_id = ?1",
                    )?;
                    statement
                        .query_map([&row.id], |policy| Ok((policy.get(0)?, policy.get(1)?)))?
                        .collect()
                })?;
                Ok(SkillInstallation {
                    description: front_matter_field(&row.markdown, "description").unwrap_or_default(),
                    id: row.id,
                    skill_id: row.skill_id,
                    name: row.name,
                    source: row.source,
                    slug: row.slug,
                    scope: SkillScope::parse(&row.scope)?,
                    repository_id: row.repository_id,
                    task_id: row.task_id,
                    install_path: row.path.into(),
                    pinned_hash: row.hash,
                    skill_markdown: row.markdown,
                    enabled: row.enabled,
                    blocked: row.trust == "blocked",
                    allow_claude: policies
                        .iter()
                        .any(|(adapter, allowed)| adapter == "claude" && *allowed),
                    allow_codex: policies
                        .iter()
                        .any(|(adapter, allowed)| adapter == "codex" && *allowed),
                    installed_at: row.installed_at,
                })
            })
            .collect()
    }

    /// Uninstall. The files go only when no other installation shares them.
    pub fn remove(&self, installation_id: &str) -> Result<(), SkillError> {
        let path: Option<String> = self.storage.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT install_path FROM skill_installations WHERE id = ?1",
                    [installation_id],
                    |row| row.get(0),
                )
                .optional()
        })?;
        let Some(path) = path else {
            return Err(SkillError::InstallationNotFound(installation_id.to_owned()));
        };
        let path = PathBuf::from(path);
        if !path.starts_with(&self.install_root) {
            return Err(SkillError::UnsafePath(path.display().to_string()));
        }
        let shared: i64 = self.storage.with_transaction(|transaction| {
            transaction.execute(
                "DELETE FROM skill_installations WHERE id = ?1",
                [installation_id],
            )?;
            transaction.query_row(
                "SELECT COUNT(*) FROM skill_installations WHERE install_path = ?1",
                [path.to_string_lossy()],
                |row| row.get(0),
            )
        })?;
        if shared == 0 && path.exists() {
            // Moved aside first so a half-deleted directory is never at a
            // path a later install would reuse.
            let trash = self
                .install_root
                .join(format!(".remove-{}", Uuid::now_v7()));
            fs::rename(&path, &trash).map_err(|source| SkillError::Filesystem {
                path: path.clone(),
                source,
            })?;
            fs::remove_dir_all(&trash).map_err(|source| SkillError::Filesystem {
                path: trash,
                source,
            })?;
        }
        Ok(())
    }
}

fn set_policy(
    transaction: &Transaction<'_>,
    installation_id: &str,
    adapter: AdapterPolicy,
    allowed: bool,
) -> rusqlite::Result<()> {
    transaction.execute(
        "INSERT INTO skill_agent_policies (installation_id, adapter_kind, allowed)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(installation_id, adapter_kind) DO UPDATE SET allowed = excluded.allowed",
        params![installation_id, adapter.as_str(), allowed],
    )?;
    Ok(())
}

fn validate_scope(
    scope: SkillScope,
    repository_id: Option<&str>,
    task_id: Option<&str>,
) -> Result<(), SkillError> {
    if matches!(
        scope,
        SkillScope::Project | SkillScope::Task | SkillScope::Workflow
    ) && repository_id.is_none()
    {
        return Err(SkillError::MissingScopeTarget {
            scope: scope.as_str().into(),
            field: "repository",
        });
    }
    if matches!(scope, SkillScope::Task | SkillScope::Workflow) && task_id.is_none() {
        return Err(SkillError::MissingScopeTarget {
            scope: scope.as_str().into(),
            field: "task",
        });
    }
    Ok(())
}

fn validate_snapshot(snapshot: &SkillSnapshot) -> Result<(), SkillError> {
    if snapshot.files.is_empty() {
        return Err(SkillError::MissingSkillMarkdown);
    }
    for file in &snapshot.files {
        let path = Path::new(&file.path);
        if path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(SkillError::UnsafePath(file.path.clone()));
        }
    }
    if !snapshot
        .files
        .iter()
        .any(|file| file.path == "SKILL.md" || file.path.ends_with("/SKILL.md"))
    {
        return Err(SkillError::MissingSkillMarkdown);
    }
    Ok(())
}

fn skill_markdown_of(files: &[SkillFile]) -> Result<String, SkillError> {
    files
        .iter()
        .filter(|file| file.path == "SKILL.md" || file.path.ends_with("/SKILL.md"))
        .min_by_key(|file| file.path.len())
        .map(|file| file.contents.clone())
        .ok_or(SkillError::MissingSkillMarkdown)
}

/// Mars's own content hash: what an installation pins.
fn snapshot_hash(files: &[SkillFile]) -> String {
    let mut hasher = Sha256::new();
    let mut ordered = files.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| left.path.cmp(&right.path));
    for file in ordered {
        hasher.update(file.path.as_bytes());
        hasher.update(file.contents.as_bytes());
    }
    hex(&hasher.finalize())
}

/// The digest skills.sh publishes with a download: SHA-256 over the files in
/// case-insensitive path order, each as its path, a NUL, its contents and a
/// NUL. Matching it shows the files arrived as the registry holds them.
#[must_use]
pub fn registry_digest(files: &[SkillFile]) -> String {
    let mut ordered = files.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        left.path
            .to_lowercase()
            .cmp(&right.path.to_lowercase())
            .then_with(|| left.path.cmp(&right.path))
    });
    let mut hasher = Sha256::new();
    for file in ordered {
        hasher.update(file.path.as_bytes());
        hasher.update([0]);
        hasher.update(file.contents.as_bytes());
        hasher.update([0]);
    }
    hex(&hasher.finalize())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

/// A field from `SKILL.md`'s YAML front matter, read without a YAML parser:
/// a skill's `name` and `description` are single-line scalars by convention.
#[must_use]
pub fn front_matter_field(markdown: &str, field: &str) -> Option<String> {
    let mut lines = markdown.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        let trimmed = line.trim_end();
        if trimmed == "---" {
            break;
        }
        if let Some(value) = trimmed.strip_prefix(&format!("{field}:")) {
            let value = value
                .trim()
                .trim_matches(|character| character == '"' || character == '\'');
            return (!value.is_empty()).then(|| value.to_owned());
        }
    }
    None
}

fn changed_paths(before: &[SkillFile], after: &[SkillFile]) -> Vec<String> {
    let before = before
        .iter()
        .map(|file| (file.path.as_str(), file.contents.as_str()))
        .collect::<BTreeMap<_, _>>();
    let after = after
        .iter()
        .map(|file| (file.path.as_str(), file.contents.as_str()))
        .collect::<BTreeMap<_, _>>();
    before
        .keys()
        .chain(after.keys())
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|path| before.get(path) != after.get(path))
        .map(str::to_owned)
        .collect()
}

/// The files of an installed version, read back from disk for a comparison.
fn read_installed_files(root: &Path) -> std::io::Result<Vec<SkillFile>> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_owned()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                pending.push(path);
            } else if let Ok(contents) = fs::read_to_string(&path) {
                let relative = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                files.push(SkillFile {
                    path: relative,
                    contents,
                });
            }
        }
    }
    Ok(files)
}

fn install_snapshot(
    install_root: &Path,
    final_path: &Path,
    snapshot: &SkillSnapshot,
) -> Result<bool, SkillError> {
    fs::create_dir_all(install_root).map_err(|source| SkillError::Filesystem {
        path: install_root.to_owned(),
        source,
    })?;
    let staging = install_root.join(format!(".staging-{}", Uuid::now_v7()));
    let result = (|| {
        for skill_file in &snapshot.files {
            let path = staging.join(&skill_file.path);
            let parent = path.parent().expect("validated relative file has a parent");
            fs::create_dir_all(parent).map_err(|source| SkillError::Filesystem {
                path: parent.to_owned(),
                source,
            })?;
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)
                .map_err(|source| SkillError::Filesystem {
                    path: path.clone(),
                    source,
                })?;
            file.write_all(skill_file.contents.as_bytes())
                .and_then(|()| file.sync_all())
                .map_err(|source| SkillError::Filesystem {
                    path: path.clone(),
                    source,
                })?;
        }
        let parent = final_path.parent().expect("install path has a parent");
        fs::create_dir_all(parent).map_err(|source| SkillError::Filesystem {
            path: parent.to_owned(),
            source,
        })?;
        fs::rename(&staging, final_path).map_err(|source| SkillError::Filesystem {
            path: final_path.to_owned(),
            source,
        })?;
        Ok(true)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn now() -> Result<String, time::error::Format> {
    OffsetDateTime::now_utc().format(&Rfc3339)
}
