//! Classifying a shell command by what it can do.
//!
//! The broker asks a different question of the human depending on this answer,
//! so the classifier fails towards the more dangerous reading: an unrecognised
//! program is `Mutating`, never `ReadOnly`, and a pipeline takes the class of
//! its most dangerous segment. Being wrong in the safe direction costs a prompt;
//! being wrong in the other direction costs a repository.

use serde::{Deserialize, Serialize};

/// What a command is capable of, in increasing order of consequence.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandClass {
    /// Observes the working tree and reports. Leaves nothing behind.
    ReadOnly,
    /// Changes files, state, or history in a recoverable way.
    Mutating,
    /// Discards work irreversibly, or reaches outside the worktree.
    Destructive,
    /// Talks to the network.
    Network,
}

impl CommandClass {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::Mutating => "mutating",
            Self::Destructive => "destructive",
            Self::Network => "network",
        }
    }

    /// Ordering for the capability ceiling. `Network` is not more dangerous than
    /// `Destructive`, it is a different axis, so severity is explicit rather
    /// than derived from the enum order.
    #[must_use]
    pub const fn severity(self) -> u8 {
        match self {
            Self::ReadOnly => 0,
            Self::Network => 1,
            Self::Mutating => 2,
            Self::Destructive => 3,
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "read_only" => Some(Self::ReadOnly),
            "mutating" => Some(Self::Mutating),
            "destructive" => Some(Self::Destructive),
            "network" => Some(Self::Network),
            _ => None,
        }
    }
}

/// One classified command, with the reason a human can read.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandAnalysis {
    pub command: String,
    pub program: String,
    pub arguments: Vec<String>,
    pub class: CommandClass,
    pub reason: String,
    /// The pattern to offer when the human picks "allow for this pattern".
    pub suggested_pattern: String,
}

/// Programs that only read. A subcommand-taking program is listed by its pair
/// so that `git status` is read-only while `git push` is not.
const READ_ONLY_PROGRAMS: &[&str] = &[
    "cat", "echo", "false", "file", "find", "grep", "head", "less", "ls", "printenv", "pwd", "rg",
    "stat", "tail", "tree", "true", "type", "wc", "whereis", "which", "whoami",
];

const READ_ONLY_SUBCOMMANDS: &[(&str, &str)] = &[
    ("cargo", "tree"),
    ("git", "blame"),
    ("git", "branch"),
    ("git", "diff"),
    ("git", "log"),
    ("git", "ls-files"),
    ("git", "rev-parse"),
    ("git", "show"),
    ("git", "status"),
    ("npm", "ls"),
    ("pnpm", "list"),
    ("pnpm", "why"),
    ("docker", "ps"),
];

/// Programs that reach the network whatever else they do.
const NETWORK_PROGRAMS: &[&str] = &[
    "curl", "nc", "ping", "rsync", "scp", "sftp", "ssh", "telnet", "wget",
];

const NETWORK_SUBCOMMANDS: &[(&str, &str)] = &[
    ("cargo", "add"),
    ("cargo", "publish"),
    ("git", "clone"),
    ("git", "fetch"),
    ("git", "pull"),
    ("git", "push"),
    ("gh", "pr"),
    ("gh", "issue"),
    ("npm", "ci"),
    ("npm", "install"),
    ("npm", "publish"),
    ("pip", "install"),
    ("pnpm", "add"),
    ("pnpm", "install"),
    ("yarn", "add"),
    ("yarn", "install"),
    ("bun", "install"),
    ("uv", "sync"),
];

/// Programs that destroy work, whatever their arguments.
const DESTRUCTIVE_PROGRAMS: &[&str] = &[
    "dd", "mkfs", "shutdown", "reboot", "sudo", "doas", "killall", "format",
];

const DESTRUCTIVE_SUBCOMMANDS: &[(&str, &str)] = &[("git", "clean"), ("docker", "system")];

/// Classify one command line, including pipelines and `&&` chains.
#[must_use]
pub fn classify_command(command: &str) -> CommandAnalysis {
    let trimmed = command.trim();
    let segments = split_segments(trimmed);
    let mut worst: Option<CommandAnalysis> = None;

    for segment in &segments {
        let analysis = classify_segment(segment);
        worst = Some(match worst {
            Some(current) if current.class.severity() >= analysis.class.severity() => current,
            _ => analysis,
        });
    }

    let mut analysis = worst.unwrap_or_else(|| CommandAnalysis {
        command: trimmed.to_owned(),
        program: String::new(),
        arguments: Vec::new(),
        class: CommandClass::Mutating,
        reason: "an empty command cannot be shown to be safe".into(),
        suggested_pattern: trimmed.to_owned(),
    });
    analysis.command = trimmed.to_owned();
    if segments.len() > 1 {
        analysis.reason = format!(
            "{} (most consequential of {} chained commands)",
            analysis.reason,
            segments.len()
        );
        analysis.suggested_pattern = trimmed.to_owned();
    }
    analysis
}

fn classify_segment(segment: &str) -> CommandAnalysis {
    let tokens = tokenize(segment);
    let Some((program_token, argument_tokens)) = tokens.split_first() else {
        return CommandAnalysis {
            command: segment.to_owned(),
            program: String::new(),
            arguments: Vec::new(),
            class: CommandClass::Mutating,
            reason: "an empty command cannot be shown to be safe".into(),
            suggested_pattern: segment.to_owned(),
        };
    };
    let program = normalize_program(program_token);
    let arguments: Vec<String> = argument_tokens.to_vec();
    let subcommand = arguments
        .iter()
        .find(|argument| !argument.starts_with('-'))
        .cloned()
        .unwrap_or_default();
    let suggested_pattern = suggest_pattern(&program, &subcommand);

    let mut analysis = CommandAnalysis {
        command: segment.to_owned(),
        program: program.clone(),
        arguments: arguments.clone(),
        class: CommandClass::Mutating,
        reason: format!("`{program}` is not a known read-only command"),
        suggested_pattern,
    };

    // Redirection writes a file no matter how harmless the program looks.
    if segment.contains('>') {
        analysis.class = CommandClass::Mutating;
        analysis.reason = "the command redirects output into a file".into();
        return analysis;
    }

    if DESTRUCTIVE_PROGRAMS.contains(&program.as_str()) {
        analysis.class = CommandClass::Destructive;
        analysis.reason = format!("`{program}` can destroy work outside the worktree");
        return analysis;
    }
    if DESTRUCTIVE_SUBCOMMANDS
        .iter()
        .any(|(candidate, sub)| *candidate == program && *sub == subcommand)
    {
        analysis.class = CommandClass::Destructive;
        analysis.reason = format!("`{program} {subcommand}` discards work irreversibly");
        return analysis;
    }
    if let Some(reason) = destructive_by_flag(&program, &subcommand, &arguments) {
        analysis.class = CommandClass::Destructive;
        analysis.reason = reason;
        return analysis;
    }

    if NETWORK_PROGRAMS.contains(&program.as_str()) {
        analysis.class = CommandClass::Network;
        analysis.reason = format!("`{program}` contacts the network");
        return analysis;
    }
    if NETWORK_SUBCOMMANDS
        .iter()
        .any(|(candidate, sub)| *candidate == program && *sub == subcommand)
    {
        analysis.class = CommandClass::Network;
        analysis.reason = format!("`{program} {subcommand}` contacts the network");
        return analysis;
    }

    if READ_ONLY_PROGRAMS.contains(&program.as_str()) {
        analysis.class = CommandClass::ReadOnly;
        analysis.reason = format!("`{program}` only reads");
        return analysis;
    }
    if READ_ONLY_SUBCOMMANDS
        .iter()
        .any(|(candidate, sub)| *candidate == program && *sub == subcommand)
    {
        analysis.class = CommandClass::ReadOnly;
        analysis.reason = format!("`{program} {subcommand}` only reads");
        return analysis;
    }

    analysis
}

fn destructive_by_flag(program: &str, subcommand: &str, arguments: &[String]) -> Option<String> {
    let has = |flag: &str| arguments.iter().any(|argument| argument == flag);
    match (program, subcommand) {
        ("rm", _) if arguments.iter().any(|argument| is_recursive_rm(argument)) => {
            Some("`rm` is being asked to delete a directory tree".into())
        }
        ("rm", _) => Some("`rm` deletes files".into()),
        ("git", "reset") if has("--hard") => {
            Some("`git reset --hard` discards uncommitted work".into())
        }
        ("git", "checkout") if has("--force") || has("-f") => {
            Some("`git checkout --force` discards uncommitted work".into())
        }
        ("git", "branch") if has("-D") => {
            Some("`git branch -D` deletes a branch that may not be merged".into())
        }
        ("git", "push") if has("--force") || has("-f") => {
            Some("`git push --force` can overwrite published history".into())
        }
        ("chmod" | "chown", _) if has("-R") => {
            Some("a recursive permission change affects every file below the path".into())
        }
        _ => None,
    }
}

fn is_recursive_rm(argument: &str) -> bool {
    argument == "--recursive"
        || (argument.starts_with('-')
            && !argument.starts_with("--")
            && argument.contains('r')
            && argument.len() > 1)
        || (argument.starts_with('-')
            && !argument.starts_with("--")
            && argument.contains('R')
            && argument.len() > 1)
}

fn suggest_pattern(program: &str, subcommand: &str) -> String {
    const MULTIPLEXERS: &[&str] = &[
        "cargo", "docker", "gh", "git", "go", "npm", "npx", "pip", "pnpm", "yarn",
    ];
    if MULTIPLEXERS.contains(&program) && !subcommand.is_empty() {
        format!("{program} {subcommand} *")
    } else if program.is_empty() {
        "*".into()
    } else {
        format!("{program} *")
    }
}

/// Split a command line on the operators that start a new command.
fn split_segments(command: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut characters = command.chars().peekable();

    while let Some(character) = characters.next() {
        match (quote, character) {
            (Some(open), _) if character == open => {
                quote = None;
                current.push(character);
            }
            (Some(_), _) => current.push(character),
            (None, '\'' | '"') => {
                quote = Some(character);
                current.push(character);
            }
            (None, '&') if characters.peek() == Some(&'&') => {
                characters.next();
                push_segment(&mut segments, &mut current);
            }
            (None, '|') if characters.peek() == Some(&'|') => {
                characters.next();
                push_segment(&mut segments, &mut current);
            }
            (None, ';' | '|' | '\n') => push_segment(&mut segments, &mut current),
            (None, _) => current.push(character),
        }
    }
    push_segment(&mut segments, &mut current);
    segments
}

fn push_segment(segments: &mut Vec<String>, current: &mut String) {
    let trimmed = current.trim().to_owned();
    current.clear();
    if !trimmed.is_empty() {
        segments.push(trimmed);
    }
}

fn tokenize(segment: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;

    for character in segment.chars() {
        match (quote, character) {
            (Some(open), _) if character == open => quote = None,
            (Some(_), _) => current.push(character),
            (None, '\'' | '"') => quote = Some(character),
            (None, character) if character.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            (None, _) => current.push(character),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// Reduce `/usr/bin/git` or `git.exe` to `git` so the tables above can match it.
fn normalize_program(token: &str) -> String {
    let without_directory = token
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(token)
        .to_ascii_lowercase();
    without_directory
        .strip_suffix(".exe")
        .or_else(|| without_directory.strip_suffix(".cmd"))
        .or_else(|| without_directory.strip_suffix(".bat"))
        .unwrap_or(&without_directory)
        .to_owned()
}
