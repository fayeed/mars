use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Command;

use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl GitOutput {
    #[must_use]
    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).trim().to_owned()
    }
}

#[derive(Debug, Error)]
pub enum GitCommandError {
    #[error("failed to launch Git in {cwd}: {source}")]
    Launch {
        cwd: String,
        #[source]
        source: std::io::Error,
    },
    #[error("Git command failed with exit {exit_code:?}: {stderr}")]
    Failed {
        exit_code: Option<i32>,
        stderr: String,
    },
}

pub fn run<I, S>(cwd: &Path, arguments: I) -> Result<GitOutput, GitCommandError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let arguments: Vec<OsString> = arguments
        .into_iter()
        .map(|argument| argument.as_ref().to_owned())
        .collect();
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(&arguments)
        .output()
        .map_err(|source| GitCommandError::Launch {
            cwd: cwd.display().to_string(),
            source,
        })?;
    if !output.status.success() {
        return Err(GitCommandError::Failed {
            exit_code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(GitOutput {
        stdout: output.stdout,
        stderr: output.stderr,
    })
}
