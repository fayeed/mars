//! Skills that are already on this machine, outside Mars.
//!
//! A repository can commit its own skills, and a person can keep skills in
//! their home folder for the CLIs to find. The CLIs load those themselves —
//! Claude Code from `.claude/skills`, Codex from `.agents/skills` and its home
//! — so Mars does not install or pin them. It lists them, so a person can see
//! everything that will shape an agent's work in one place, and says which
//! agent reads each.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::install::front_matter_field;

/// Where a skill found on disk lives, which decides which agents load it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalOrigin {
    /// `.claude/skills` in the repository: Claude Code loads it in every
    /// worktree, because the folder is committed.
    RepositoryClaude,
    /// `.agents/skills` in the repository, which Codex reads.
    RepositoryAgents,
    /// A plain `skills/` folder in the repository: shared instructions no CLI
    /// loads on its own.
    RepositoryShared,
    /// `~/.claude/skills`, which Claude Code loads everywhere.
    UserClaude,
    /// `~/.codex/skills`, which Codex loads everywhere.
    UserCodex,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSkill {
    pub name: String,
    pub description: String,
    pub origin: LocalOrigin,
    /// The skill's folder.
    pub path: PathBuf,
    pub skill_markdown: String,
    /// Which agents load it without Mars doing anything.
    pub loaded_by: Vec<&'static str>,
}

/// Every skill in the well-known folders of `repository_root` and `home`.
#[must_use]
pub fn discover_local(repository_root: &Path, home: Option<&Path>) -> Vec<LocalSkill> {
    let mut places: Vec<(PathBuf, LocalOrigin)> = vec![
        (
            repository_root.join(".claude").join("skills"),
            LocalOrigin::RepositoryClaude,
        ),
        (
            repository_root.join(".agents").join("skills"),
            LocalOrigin::RepositoryAgents,
        ),
        (
            repository_root.join("skills"),
            LocalOrigin::RepositoryShared,
        ),
    ];
    if let Some(home) = home {
        places.push((home.join(".claude").join("skills"), LocalOrigin::UserClaude));
        places.push((home.join(".codex").join("skills"), LocalOrigin::UserCodex));
    }
    let mut found = Vec::new();
    for (folder, origin) in places {
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        let mut skills: Vec<LocalSkill> = entries
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            // Codex keeps its own built-in skills under `.system`.
            .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
            .filter_map(|entry| {
                let markdown = fs::read_to_string(entry.path().join("SKILL.md")).ok()?;
                let folder_name = entry.file_name().to_string_lossy().into_owned();
                Some(LocalSkill {
                    name: front_matter_field(&markdown, "name").unwrap_or(folder_name),
                    description: front_matter_field(&markdown, "description").unwrap_or_default(),
                    origin,
                    path: entry.path(),
                    skill_markdown: markdown,
                    loaded_by: match origin {
                        LocalOrigin::RepositoryClaude | LocalOrigin::UserClaude => vec!["claude"],
                        LocalOrigin::RepositoryAgents | LocalOrigin::UserCodex => vec!["codex"],
                        LocalOrigin::RepositoryShared => Vec::new(),
                    },
                })
            })
            .collect();
        skills.sort_by(|left, right| left.name.cmp(&right.name));
        found.extend(skills);
    }
    found
}
