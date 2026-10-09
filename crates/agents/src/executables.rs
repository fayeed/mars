//! Finding the agent CLIs a person has installed.
//!
//! Spawning the bare name `claude` only works when the CLI is on the `PATH`
//! Mars inherited, and only for a real executable: on Windows an npm install is
//! a `claude.cmd` shim that a bare-name spawn never finds, and an app opened
//! from the macOS Finder does not inherit the shell's `PATH` at all. So the
//! search is explicit, ordered, and reported: a path the person configured,
//! then `PATH` honouring `PATHEXT`, then the places each CLI's own installers
//! put it, then the copies an editor extension ships. The first that exists
//! wins, and the answer says where it came from so Settings can show it.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;

use crate::adapter::AdapterKind;

/// Where a resolved executable was found.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutableSource {
    /// The person chose this path in Settings.
    Configured,
    /// Found on the `PATH` Mars was started with.
    Path,
    /// Where the CLI's own installer puts it.
    Installer,
    /// The copy an editor extension ships, which shares the CLI's sign-in.
    EditorExtension,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedExecutable {
    pub path: PathBuf,
    pub source: ExecutableSource,
}

/// Nothing was found; `searched` lists where, so the message can say so.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{name} was not found (looked in {} place(s))", searched.len())]
pub struct ExecutableNotFound {
    pub name: String,
    pub searched: Vec<String>,
}

/// The command a CLI is known by.
#[must_use]
pub const fn command_name(kind: AdapterKind) -> &'static str {
    match kind {
        AdapterKind::Claude => "claude",
        AdapterKind::Codex => "codex",
    }
}

/// Find `kind`'s CLI, preferring `configured` when one is given and exists.
pub fn resolve(
    kind: AdapterKind,
    configured: Option<&Path>,
) -> Result<ResolvedExecutable, ExecutableNotFound> {
    resolve_in(kind, configured, &Environment::current())
}

/// What the search reads from the machine, separated so tests can supply it.
#[derive(Clone, Debug, Default)]
pub struct Environment {
    pub path: Option<OsString>,
    pub path_ext: Option<OsString>,
    pub home: Option<PathBuf>,
    pub app_data: Option<PathBuf>,
    pub windows: bool,
}

impl Environment {
    #[must_use]
    pub fn current() -> Self {
        Self {
            path: std::env::var_os("PATH"),
            path_ext: std::env::var_os("PATHEXT"),
            home: std::env::var_os("USERPROFILE")
                .filter(|_| cfg!(windows))
                .or_else(|| std::env::var_os("HOME"))
                .map(PathBuf::from),
            app_data: std::env::var_os("APPDATA").map(PathBuf::from),
            windows: cfg!(windows),
        }
    }
}

pub fn resolve_in(
    kind: AdapterKind,
    configured: Option<&Path>,
    environment: &Environment,
) -> Result<ResolvedExecutable, ExecutableNotFound> {
    let name = command_name(kind);
    let mut searched = Vec::new();

    if let Some(configured) = configured {
        searched.push(configured.display().to_string());
        if is_executable(configured) {
            return Ok(ResolvedExecutable {
                path: configured.to_owned(),
                source: ExecutableSource::Configured,
            });
        }
    }

    let names = file_names(name, environment);
    if let Some(path) = &environment.path {
        for directory in std::env::split_paths(path) {
            for file in &names {
                let candidate = directory.join(file);
                if is_executable(&candidate) {
                    return Ok(ResolvedExecutable {
                        path: candidate,
                        source: ExecutableSource::Path,
                    });
                }
            }
        }
        searched.push("PATH".into());
    }

    for directory in installer_directories(kind, environment) {
        for file in &names {
            let candidate = directory.join(file);
            searched.push(candidate.display().to_string());
            if is_executable(&candidate) {
                return Ok(ResolvedExecutable {
                    path: candidate,
                    source: ExecutableSource::Installer,
                });
            }
        }
    }

    for candidate in editor_extension_binaries(kind, environment) {
        searched.push(candidate.display().to_string());
        if is_executable(&candidate) {
            return Ok(ResolvedExecutable {
                path: candidate,
                source: ExecutableSource::EditorExtension,
            });
        }
    }

    Err(ExecutableNotFound {
        name: name.to_owned(),
        searched,
    })
}

/// The file names a command can have here. On Windows a real `.exe` is
/// preferred over a script shim, then whatever `PATHEXT` adds.
fn file_names(name: &str, environment: &Environment) -> Vec<String> {
    if !environment.windows {
        return vec![name.to_owned()];
    }
    let mut extensions = vec![".exe".to_owned(), ".cmd".to_owned(), ".bat".to_owned()];
    if let Some(path_ext) = &environment.path_ext {
        for extension in path_ext.to_string_lossy().split(';') {
            let extension = extension.trim().to_ascii_lowercase();
            if !extension.is_empty() && !extensions.contains(&extension) {
                extensions.push(extension);
            }
        }
    }
    extensions
        .into_iter()
        // Only formats a process can be started from directly.
        .filter(|extension| matches!(extension.as_str(), ".exe" | ".cmd" | ".bat" | ".com"))
        .map(|extension| format!("{name}{extension}"))
        .collect()
}

fn installer_directories(kind: AdapterKind, environment: &Environment) -> Vec<PathBuf> {
    let mut directories = Vec::new();
    if let Some(home) = &environment.home {
        // The native installers for both CLIs.
        directories.push(home.join(".local").join("bin"));
        if kind == AdapterKind::Claude {
            // The per-user install older Claude Code versions migrated to.
            directories.push(home.join(".claude").join("local"));
        }
        directories.push(home.join(".npm-global").join("bin"));
        directories.push(home.join(".bun").join("bin"));
        directories.push(home.join(".volta").join("bin"));
        if kind == AdapterKind::Codex {
            directories.push(home.join(".cargo").join("bin"));
        }
    }
    if environment.windows {
        if let Some(app_data) = &environment.app_data {
            directories.push(app_data.join("npm"));
        }
    } else {
        directories.push(PathBuf::from("/opt/homebrew/bin"));
        directories.push(PathBuf::from("/usr/local/bin"));
        directories.push(PathBuf::from("/usr/bin"));
    }
    directories
}

/// The CLI copies editor extensions ship, newest first. Both are the vendor's
/// own unmodified binaries and read the same sign-in as the terminal CLI.
fn editor_extension_binaries(kind: AdapterKind, environment: &Environment) -> Vec<PathBuf> {
    let Some(home) = &environment.home else {
        return Vec::new();
    };
    let executable = |name: &str| {
        if environment.windows {
            format!("{name}.exe")
        } else {
            name.to_owned()
        }
    };
    let mut found: Vec<(SystemTime, PathBuf)> = Vec::new();
    for editor in [".vscode", ".vscode-insiders", ".cursor", ".windsurf"] {
        let Ok(entries) = fs::read_dir(home.join(editor).join("extensions")) else {
            continue;
        };
        for entry in entries.flatten() {
            let folder = entry.file_name().to_string_lossy().to_ascii_lowercase();
            let candidates: Vec<PathBuf> = match kind {
                AdapterKind::Claude if folder.starts_with("anthropic.claude-code-") => vec![
                    entry
                        .path()
                        .join("resources")
                        .join("native-binary")
                        .join(executable("claude")),
                ],
                AdapterKind::Codex if folder.starts_with("openai.chatgpt-") => {
                    // `bin/<platform>/codex`, one platform folder per build.
                    fs::read_dir(entry.path().join("bin"))
                        .into_iter()
                        .flatten()
                        .flatten()
                        .map(|platform| platform.path().join(executable("codex")))
                        .collect()
                }
                _ => Vec::new(),
            };
            for candidate in candidates {
                let modified = fs::metadata(&candidate)
                    .and_then(|metadata| metadata.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                found.push((modified, candidate));
            }
        }
    }
    found.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    found.into_iter().map(|(_, path)| path).collect()
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}
