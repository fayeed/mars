use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use mars_storage::{Storage, StorageError};
use rusqlite::{OptionalExtension, params};
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
    fn as_str(self) -> &'static str {
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
    pub scope: SkillScope,
    pub install_path: PathBuf,
    pub pinned_hash: String,
    pub skill_markdown: String,
    pub allow_claude: bool,
    pub allow_codex: bool,
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

    pub fn inspect(&self, candidate: &SkillCandidate) -> Result<InspectedSkill, SkillError> {
        let snapshot = self.registry.download(&candidate.source, &candidate.name)?;
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
        let skill_markdown = snapshot
            .files
            .iter()
            .filter(|file| file.path == "SKILL.md" || file.path.ends_with("/SKILL.md"))
            .min_by_key(|file| file.path.len())
            .map(|file| file.contents.clone())
            .ok_or(SkillError::MissingSkillMarkdown)?;
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
        let trust_state = previous
            .as_ref()
            .filter(|(_, _, raw)| raw == &serde_json::to_string(&snapshot).unwrap_or_default())
            .map(|(_, trust, _)| trust.as_str())
            .unwrap_or("inspected");
        let timestamp = now()?;
        let snapshot_json = serde_json::to_string(&snapshot)?;
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
                    candidate.installs as i64,
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
            trust_state: trust_state.into(),
            skill_markdown,
            files: snapshot.files.into_iter().map(|file| file.path).collect(),
            changed_files,
        })
    }

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
        let final_path = self
            .install_root
            .join(scope.as_str())
            .join(skill_id)
            .join(&hash[..12]);
        let created = if final_path.is_dir() {
            false
        } else {
            install_snapshot(&self.install_root, &final_path, &snapshot)?;
            true
        };
        let installation_id = Uuid::now_v7().to_string();
        let timestamp = now()?;
        let result = self.storage.with_transaction(|transaction| {
            transaction.execute("UPDATE skills SET trust_state = 'trusted', updated_at = ?2 WHERE id = ?1", params![skill_id, timestamp])?;
            transaction.execute(
                "INSERT INTO skill_installations
                 (id, skill_id, repository_id, task_id, scope, install_path, pinned_hash,
                  enabled, installed_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?8)
                 ON CONFLICT(skill_id, repository_id, task_id, scope) DO UPDATE SET
                  install_path = excluded.install_path, pinned_hash = excluded.pinned_hash,
                  enabled = 1, updated_at = excluded.updated_at",
                params![installation_id, skill_id, repository_id, task_id, scope.as_str(), final_path.to_string_lossy(), hash, timestamp],
            )?;
            let stored_id: String = transaction.query_row(
                "SELECT id FROM skill_installations WHERE skill_id = ?1
                 AND repository_id IS ?2 AND task_id IS ?3 AND scope = ?4",
                params![skill_id, repository_id, task_id, scope.as_str()],
                |row| row.get(0),
            )?;
            for (adapter, allowed) in [(AdapterPolicy::Claude, allow_claude), (AdapterPolicy::Codex, allow_codex)] {
                transaction.execute(
                    "INSERT INTO skill_agent_policies (installation_id, adapter_kind, allowed)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(installation_id, adapter_kind) DO UPDATE SET allowed = excluded.allowed",
                    params![stored_id, adapter.as_str(), allowed],
                )?;
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
        Ok(SkillInstallation {
            id: stored_id,
            skill_id: skill_id.to_owned(),
            name,
            scope,
            install_path: final_path,
            pinned_hash: hash,
            skill_markdown: markdown,
            allow_claude,
            allow_codex,
        })
    }

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

    pub fn list_for_task(&self, task_id: &str) -> Result<Vec<SkillInstallation>, SkillError> {
        type InstallationRow = (String, String, String, String, String, String, String);
        let rows: Vec<InstallationRow> = self.storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT i.id, i.skill_id, s.canonical_name, i.scope, i.install_path,
                        i.pinned_hash, s.skill_markdown
                 FROM skill_installations i JOIN skills s ON s.id = i.skill_id
                 WHERE i.enabled = 1 AND (i.task_id = ?1 OR i.scope = 'global')
                 ORDER BY i.installed_at DESC, i.id DESC",
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
                        row.get(6)?,
                    ))
                })?
                .collect()
        })?;
        rows.into_iter()
            .map(|(id, skill_id, name, scope, path, hash, markdown)| {
                let policies: Vec<(String, bool)> = self.storage.with_connection(|connection| {
                    let mut statement = connection.prepare(
                        "SELECT adapter_kind, allowed FROM skill_agent_policies WHERE installation_id = ?1",
                    )?;
                    statement
                        .query_map([&id], |row| Ok((row.get(0)?, row.get(1)?)))?
                        .collect()
                })?;
                Ok(SkillInstallation {
                    id,
                    skill_id,
                    name,
                    scope: SkillScope::parse(&scope)?,
                    install_path: path.into(),
                    pinned_hash: hash,
                    skill_markdown: markdown,
                    allow_claude: policies.iter().any(|(adapter, allowed)| adapter == "claude" && *allowed),
                    allow_codex: policies.iter().any(|(adapter, allowed)| adapter == "codex" && *allowed),
                })
            })
            .collect()
    }

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
        let trash = self
            .install_root
            .join(format!(".remove-{}", Uuid::now_v7()));
        if path.exists() {
            fs::rename(&path, &trash).map_err(|source| SkillError::Filesystem {
                path: path.clone(),
                source,
            })?;
        }
        let deleted = self.storage.with_connection(|connection| {
            connection.execute(
                "DELETE FROM skill_installations WHERE id = ?1",
                [installation_id],
            )
        });
        if let Err(error) = deleted {
            if trash.exists() {
                let _ = fs::rename(&trash, &path);
            }
            return Err(error.into());
        }
        if trash.exists() {
            fs::remove_dir_all(&trash).map_err(|source| SkillError::Filesystem {
                path: trash,
                source,
            })?;
        }
        Ok(())
    }
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

fn snapshot_hash(files: &[SkillFile]) -> String {
    use std::fmt::Write as _;
    let mut hasher = Sha256::new();
    let mut ordered = files.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| left.path.cmp(&right.path));
    for file in ordered {
        hasher.update(file.path.as_bytes());
        hasher.update(file.contents.as_bytes());
    }
    let digest = hasher.finalize();
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
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
