//! Which account an agent CLI will bill, as the CLI itself reports it.
//!
//! Mars never signs anyone in and never reads a provider credential. It asks
//! the installed CLI what it is signed in with (`claude auth status`,
//! `codex login status`) and checks its own environment for the variables that
//! outrank that sign-in. The second half matters because Mars runs agents
//! non-interactively: there, Claude Code uses `ANTHROPIC_API_KEY` whenever it is
//! present, without the one-time approval it asks for in a terminal, so a key
//! exported for some other project would quietly take over billing from the
//! person's plan.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::adapter::{AdapterError, AdapterKind, AgentAdapter};

/// Who pays for a run.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum Billing {
    /// The person's own subscription sign-in. `plan` is the tier when the CLI
    /// names one (`pro`, `max`, `team`, ...).
    Plan {
        plan: Option<String>,
    },
    /// An API key, named by where it came from rather than by its value.
    ApiKey {
        source: String,
    },
    /// A cloud provider such as Amazon Bedrock, configured outside Mars.
    CloudProvider {
        provider: String,
    },
    SignedOut,
    /// The CLI did not answer in a form Mars reads, for example a version that
    /// predates its status command. Not treated as signed out: Mars would
    /// rather let the CLI speak for itself at launch than refuse on a guess.
    Unknown {
        detail: String,
    },
}

impl Billing {
    #[must_use]
    pub fn is_signed_in(&self) -> bool {
        matches!(
            self,
            Self::Plan { .. } | Self::ApiKey { .. } | Self::CloudProvider { .. }
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentAccount {
    pub kind: AdapterKind,
    /// What a run Mars starts now would bill.
    pub billing: Billing,
    /// What the CLI is signed in with once `overrides` are set aside. The same
    /// as `billing` whenever nothing overrides it.
    pub sign_in: Billing,
    /// Variables in Mars's environment that outrank `sign_in` for a
    /// non-interactive run, highest precedence first.
    pub overrides: Vec<String>,
    /// Whether the person chose `sign_in` over `overrides` for this agent.
    pub use_sign_in: bool,
    /// What a person runs, in their own terminal, to sign in.
    pub sign_in_command: Option<String>,
}

impl AgentAccount {
    /// Combines what the CLI reported with the variables that outrank it.
    #[must_use]
    pub fn resolve(
        kind: AdapterKind,
        sign_in_command: Option<&str>,
        sign_in: Billing,
        mut overrides: Vec<String>,
        use_sign_in: bool,
    ) -> Self {
        // A cloud provider is chosen above every key variable, so none of them
        // is in play and none would be worth setting aside.
        if matches!(sign_in, Billing::CloudProvider { .. }) {
            overrides.clear();
        }
        let billing = match overrides.first() {
            Some(variable) if !(use_sign_in && sign_in.is_signed_in()) => Billing::ApiKey {
                source: variable.clone(),
            },
            _ => sign_in.clone(),
        };
        Self {
            kind,
            billing,
            sign_in,
            overrides,
            use_sign_in,
            sign_in_command: sign_in_command.map(str::to_owned),
        }
    }

    /// The variables to remove from an agent's environment so it runs on its
    /// sign-in. Empty unless the person asked for that and the sign-in exists:
    /// removing a key from someone who has nothing else would only turn a
    /// working setup into a signed-out one.
    #[must_use]
    pub fn removed_environment(&self) -> Vec<OsString> {
        if self.use_sign_in && self.sign_in.is_signed_in() {
            self.overrides.iter().map(OsString::from).collect()
        } else {
            Vec::new()
        }
    }
}

/// Asks `executable` what it is signed in with and resolves what a run would
/// bill. The status command runs without the overriding variables, so it
/// reports the sign-in underneath them rather than the key on top.
pub fn read_account(
    adapter: &dyn AgentAdapter,
    executable: &Path,
    use_sign_in: bool,
) -> Result<AgentAccount, AdapterError> {
    let overrides = present_variables(adapter.overriding_variables(), |name| {
        std::env::var_os(name)
    });
    let sign_in = match adapter.sign_in_status_arguments() {
        Some(arguments) => {
            let mut command = mars_core::process::command(executable);
            command.args(arguments);
            for variable in &overrides {
                command.env_remove(variable);
            }
            let output = command
                .output()
                .map_err(|source| AdapterError::ProbeLaunch {
                    path: executable.to_owned(),
                    source,
                })?;
            adapter.parse_sign_in(output.status.success(), &output.stdout, &output.stderr)
        }
        None => adapter.parse_sign_in(false, &[], &[]),
    };
    Ok(AgentAccount::resolve(
        adapter.kind(),
        adapter.sign_in_command(),
        sign_in,
        overrides,
        use_sign_in,
    ))
}

/// The names in `candidates` that are set to something non-empty, in order.
#[must_use]
pub fn present_variables(
    candidates: &[&str],
    lookup: impl Fn(&str) -> Option<OsString>,
) -> Vec<String> {
    candidates
        .iter()
        .filter(|name| lookup(name).is_some_and(|value| !value.is_empty()))
        .map(|name| (*name).to_owned())
        .collect()
}

/// The agents a person chose to run on their sign-in even where a variable
/// would outrank it.
///
/// Kept on disk because it is the decision Claude Code itself asks for once
/// and then remembers. Asking again on every start would be noise; forgetting
/// it on restart would put billing back on the key without anyone noticing.
pub struct SignInPreferences {
    path: PathBuf,
    chosen: Mutex<Vec<AdapterKind>>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredPreferences {
    use_sign_in: Vec<AdapterKind>,
}

impl SignInPreferences {
    /// Reads the file at `path`. A missing or unreadable file means nobody has
    /// chosen anything yet, which is also the safe reading of a damaged one:
    /// every agent bills as its CLI is configured to.
    #[must_use]
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let chosen = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<StoredPreferences>(&bytes).ok())
            .unwrap_or_default()
            .use_sign_in;
        Self {
            path,
            chosen: Mutex::new(chosen),
        }
    }

    #[must_use]
    pub fn uses_sign_in(&self, kind: AdapterKind) -> bool {
        self.chosen
            .lock()
            .is_ok_and(|chosen| chosen.contains(&kind))
    }

    pub fn set(&self, kind: AdapterKind, use_sign_in: bool) -> io::Result<()> {
        let mut chosen = self
            .chosen
            .lock()
            .map_err(|_| io::Error::other("sign-in preference lock was poisoned"))?;
        let mut next = chosen.clone();
        next.retain(|existing| *existing != kind);
        if use_sign_in {
            next.push(kind);
        }
        let bytes = serde_json::to_vec_pretty(&StoredPreferences {
            use_sign_in: next.clone(),
        })
        .map_err(io::Error::other)?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.path, bytes)?;
        *chosen = next;
        Ok(())
    }
}
