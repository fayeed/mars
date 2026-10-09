//! Everyday source control over a working directory: what changed, staging,
//! committing, history, branches, syncing with a remote, and bringing a task's
//! branch home.
//!
//! Everything here runs the person's own Git with argument arrays (ADR-004),
//! so their hooks, signing, credential helpers and configuration apply exactly
//! as they would in a terminal. The same functions serve the main repository
//! and a task's worktree: a directory is a directory.
//!
//! Reads are plain. Writes are explicit, one per call, and never widen what
//! was asked for: staging a file stages that file; nothing is staged as a side
//! effect of committing. Integration — merging a task's branch into its base —
//! is previewed first, fingerprinted over what it would do, and applied only
//! against that fingerprint; a conflict stops it and leaves the base exactly
//! as it was (Architecture §9, ADR-012).

use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::command::{GitCommandError, GitOutput, run};

#[derive(Debug, Error)]
pub enum ScmError {
    #[error(transparent)]
    Git(#[from] GitCommandError),
    #[error("{0}")]
    Refused(String),
    #[error("the integration preview changed: approved {approved}, now {current}")]
    PreviewChanged { approved: String, current: String },
    #[error("merging {head} into {base} stopped on conflicts in {}", files.join(", "))]
    Conflicts {
        base: String,
        head: String,
        files: Vec<String>,
    },
}

/// One changed path, as `git status` sees it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub path: String,
    /// Where a rename or copy came from.
    pub original_path: Option<String>,
    /// What the index holds relative to HEAD: `M`, `A`, `D`, `R`, `C`, `T`,
    /// or `.` for nothing staged.
    pub staged: char,
    /// What the working tree holds relative to the index, in the same letters.
    pub unstaged: char,
    pub untracked: bool,
    /// Both sides changed it in a merge that has not been resolved.
    pub conflicted: bool,
}

/// What a working directory looks like right now.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoStatus {
    pub root: PathBuf,
    /// `None` when HEAD is detached.
    pub branch: Option<String>,
    pub head_oid: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub changes: Vec<FileChange>,
    /// A merge, rebase, cherry-pick or revert is part-way through.
    pub operation: Option<String>,
}

impl RepoStatus {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.changes.is_empty()
    }
}

/// Read the status of `root`.
pub fn status(root: &Path) -> Result<RepoStatus, ScmError> {
    let output = run(
        root,
        [
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
        ],
    )?;
    let mut status = RepoStatus {
        root: root.to_owned(),
        branch: None,
        head_oid: None,
        upstream: None,
        ahead: 0,
        behind: 0,
        changes: Vec::new(),
        operation: in_progress_operation(root),
    };
    let records: Vec<&[u8]> = output.stdout.split(|byte| *byte == 0).collect();
    let mut index = 0;
    while index < records.len() {
        let record = String::from_utf8_lossy(records[index]).into_owned();
        index += 1;
        if record.is_empty() {
            continue;
        }
        if let Some(header) = record.strip_prefix("# ") {
            let (key, value) = header.split_once(' ').unwrap_or((header, ""));
            match key {
                "branch.oid" if value != "(initial)" => status.head_oid = Some(value.to_owned()),
                "branch.head" if value != "(detached)" => status.branch = Some(value.to_owned()),
                "branch.upstream" => status.upstream = Some(value.to_owned()),
                "branch.ab" => {
                    for part in value.split_whitespace() {
                        if let Some(ahead) = part.strip_prefix('+') {
                            status.ahead = ahead.parse().unwrap_or(0);
                        } else if let Some(behind) = part.strip_prefix('-') {
                            status.behind = behind.parse().unwrap_or(0);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        let mut fields = record.splitn(2, ' ');
        let kind = fields.next().unwrap_or_default();
        let rest = fields.next().unwrap_or_default();
        match kind {
            // `1 XY sub mH mI mW hH hI path`
            "1" => {
                let parts: Vec<&str> = rest.splitn(8, ' ').collect();
                if parts.len() == 8 {
                    let xy: Vec<char> = parts[0].chars().collect();
                    status.changes.push(FileChange {
                        path: parts[7].to_owned(),
                        original_path: None,
                        staged: xy.first().copied().unwrap_or('.'),
                        unstaged: xy.get(1).copied().unwrap_or('.'),
                        untracked: false,
                        conflicted: false,
                    });
                }
            }
            // `2 XY sub mH mI mW hH hI Xscore path` then the original path.
            "2" => {
                let parts: Vec<&str> = rest.splitn(9, ' ').collect();
                let original = records
                    .get(index)
                    .map(|bytes| String::from_utf8_lossy(bytes).into_owned());
                index += 1;
                if parts.len() == 9 {
                    let xy: Vec<char> = parts[0].chars().collect();
                    status.changes.push(FileChange {
                        path: parts[8].to_owned(),
                        original_path: original,
                        staged: xy.first().copied().unwrap_or('.'),
                        unstaged: xy.get(1).copied().unwrap_or('.'),
                        untracked: false,
                        conflicted: false,
                    });
                }
            }
            // `u XY sub m1 m2 m3 mW h1 h2 h3 path`
            "u" => {
                let parts: Vec<&str> = rest.splitn(10, ' ').collect();
                if parts.len() == 10 {
                    let xy: Vec<char> = parts[0].chars().collect();
                    status.changes.push(FileChange {
                        path: parts[9].to_owned(),
                        original_path: None,
                        staged: xy.first().copied().unwrap_or('U'),
                        unstaged: xy.get(1).copied().unwrap_or('U'),
                        untracked: false,
                        conflicted: true,
                    });
                }
            }
            "?" => status.changes.push(FileChange {
                path: rest.to_owned(),
                original_path: None,
                staged: '.',
                unstaged: '?',
                untracked: true,
                conflicted: false,
            }),
            _ => {}
        }
    }
    Ok(status)
}

fn in_progress_operation(root: &Path) -> Option<String> {
    let git_dir = run(root, ["rev-parse", "--absolute-git-dir"])
        .ok()
        .map(|output| PathBuf::from(output.stdout_text()))?;
    [
        ("MERGE_HEAD", "merge"),
        ("rebase-merge", "rebase"),
        ("rebase-apply", "rebase"),
        ("CHERRY_PICK_HEAD", "cherry-pick"),
        ("REVERT_HEAD", "revert"),
    ]
    .into_iter()
    .find(|(marker, _)| git_dir.join(marker).exists())
    .map(|(_, operation)| operation.to_owned())
}

/// The unified diff of one path or of everything. `staged` reads the index
/// against HEAD; otherwise the working tree against the index. An untracked
/// file is shown as wholly added.
pub fn diff(root: &Path, path: Option<&str>, staged: bool) -> Result<String, ScmError> {
    if let Some(path) = path {
        let untracked = !staged
            && run(root, ["ls-files", "--error-unmatch", "--", path]).is_err()
            && root.join(path).is_file();
        if untracked {
            // `--no-index` exits 1 when the files differ, which they always do.
            let output = mars_core::process::command("git")
                .arg("-C")
                .arg(root)
                .args([
                    "diff",
                    "--no-ext-diff",
                    "--no-index",
                    "--",
                    null_device(),
                    path,
                ])
                .output()
                .map_err(|source| GitCommandError::Launch {
                    cwd: root.display().to_string(),
                    source,
                })?;
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }
    }
    let mut arguments = vec!["diff", "--no-ext-diff", "--unified=3"];
    if staged {
        arguments.push("--cached");
    }
    arguments.push("--");
    if let Some(path) = path {
        arguments.push(path);
    }
    Ok(text(&run(root, arguments)?))
}

const fn null_device() -> &'static str {
    if cfg!(windows) { "NUL" } else { "/dev/null" }
}

fn text(output: &GitOutput) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn require_paths(paths: &[String]) -> Result<(), ScmError> {
    if paths.is_empty() {
        return Err(ScmError::Refused("no files were named".into()));
    }
    if paths
        .iter()
        .any(|path| path.is_empty() || path.starts_with('-') || Path::new(path).is_absolute())
    {
        return Err(ScmError::Refused(
            "paths must be relative to the repository".into(),
        ));
    }
    Ok(())
}

/// Stage exactly these paths.
pub fn stage(root: &Path, paths: &[String]) -> Result<(), ScmError> {
    require_paths(paths)?;
    let mut arguments = vec!["add".to_owned(), "--".to_owned()];
    arguments.extend(paths.iter().cloned());
    run(root, arguments)?;
    Ok(())
}

/// Take exactly these paths out of the index, leaving the working tree alone.
pub fn unstage(root: &Path, paths: &[String]) -> Result<(), ScmError> {
    require_paths(paths)?;
    let has_head = run(root, ["rev-parse", "--verify", "--quiet", "HEAD"]).is_ok();
    let mut arguments: Vec<String> = if has_head {
        vec!["restore".into(), "--staged".into(), "--".into()]
    } else {
        // Before the first commit there is no HEAD to restore from.
        vec!["rm".into(), "--cached".into(), "-q".into(), "--".into()]
    };
    arguments.extend(paths.iter().cloned());
    run(root, arguments)?;
    Ok(())
}

/// Throw away working-tree changes to these paths. Tracked files go back to
/// what the index holds; untracked files are deleted. Irreversible, which is
/// why the screen asks first.
pub fn discard(root: &Path, paths: &[String]) -> Result<(), ScmError> {
    require_paths(paths)?;
    let status = status(root)?;
    let (untracked, tracked): (Vec<&String>, Vec<&String>) = paths.iter().partition(|path| {
        status
            .changes
            .iter()
            .any(|change| change.untracked && &change.path == *path)
    });
    if !tracked.is_empty() {
        let mut arguments = vec![
            "restore".to_owned(),
            "--worktree".to_owned(),
            "--".to_owned(),
        ];
        arguments.extend(tracked.into_iter().cloned());
        run(root, arguments)?;
    }
    if !untracked.is_empty() {
        let mut arguments = vec![
            "clean".to_owned(),
            "-f".to_owned(),
            "-q".to_owned(),
            "--".to_owned(),
        ];
        arguments.extend(untracked.into_iter().cloned());
        run(root, arguments)?;
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitInfo {
    pub oid: String,
    pub short_oid: String,
    pub subject: String,
    pub author: String,
    pub authored_at: String,
    /// Branch and tag names pointing here.
    pub refs: Vec<String>,
}

/// Commit what is staged. Nothing is staged on the person's behalf: an empty
/// index is refused rather than turned into "commit everything".
pub fn commit(root: &Path, message: &str, amend: bool) -> Result<CommitInfo, ScmError> {
    let message = message.trim();
    if message.is_empty() {
        return Err(ScmError::Refused("a commit needs a message".into()));
    }
    if !amend {
        let staged = status(root)?
            .changes
            .iter()
            .any(|change| change.staged != '.' && !change.untracked);
        if !staged {
            return Err(ScmError::Refused(
                "nothing is staged; stage the files this commit should contain".into(),
            ));
        }
    }
    let mut arguments = vec!["commit", "-m", message];
    if amend {
        arguments.push("--amend");
    }
    run(root, arguments)?;
    log(root, None, 1, 0)?
        .into_iter()
        .next()
        .ok_or_else(|| ScmError::Refused("the commit could not be read back".into()))
}

const FIELD: char = '\u{1f}';
const RECORD: char = '\u{1e}';

/// Recent commits, newest first. `revision` defaults to HEAD.
pub fn log(
    root: &Path,
    revision: Option<&str>,
    limit: usize,
    skip: usize,
) -> Result<Vec<CommitInfo>, ScmError> {
    if run(root, ["rev-parse", "--verify", "--quiet", "HEAD"]).is_err() && revision.is_none() {
        return Ok(Vec::new());
    }
    let format = format!("--format=%H{FIELD}%h{FIELD}%s{FIELD}%an{FIELD}%aI{FIELD}%D{RECORD}");
    let limit = format!("--max-count={}", limit.clamp(1, 500));
    let skip = format!("--skip={skip}");
    let mut arguments = vec!["log", format.as_str(), limit.as_str(), skip.as_str()];
    if let Some(revision) = revision {
        if revision.starts_with('-') {
            return Err(ScmError::Refused("not a revision".into()));
        }
        arguments.push(revision);
    }
    arguments.push("--");
    let output = text(&run(root, arguments)?);
    Ok(output
        .split(RECORD)
        .filter_map(|record| {
            let record = record.trim_start_matches(['\n', '\r']);
            if record.is_empty() {
                return None;
            }
            let fields: Vec<&str> = record.split(FIELD).collect();
            if fields.len() < 6 {
                return None;
            }
            Some(CommitInfo {
                oid: fields[0].to_owned(),
                short_oid: fields[1].to_owned(),
                subject: fields[2].to_owned(),
                author: fields[3].to_owned(),
                authored_at: fields[4].to_owned(),
                refs: fields[5]
                    .split(", ")
                    .map(|reference| reference.trim_start_matches("HEAD -> ").to_owned())
                    .filter(|reference| !reference.is_empty())
                    .collect(),
            })
        })
        .collect())
}

/// One commit's change, as a unified diff.
pub fn show(root: &Path, oid: &str) -> Result<String, ScmError> {
    if !oid.chars().all(|character| character.is_ascii_hexdigit()) || oid.len() < 4 {
        return Err(ScmError::Refused(format!("`{oid}` is not a commit id")));
    }
    Ok(text(&run(
        root,
        [
            "show",
            "--no-ext-diff",
            "--unified=3",
            "--format=",
            "--first-parent",
            oid,
        ],
    )?))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchInfo {
    pub name: String,
    pub current: bool,
    pub upstream: Option<String>,
    pub oid: String,
    pub subject: String,
    pub committed_at: String,
    /// Checked out in another worktree, which Git will not switch to here.
    pub checked_out_elsewhere: bool,
}

/// Local branches, most recently committed first.
pub fn branches(root: &Path) -> Result<Vec<BranchInfo>, ScmError> {
    let format = format!(
        "--format=%(refname:short){FIELD}%(HEAD){FIELD}%(upstream:short){FIELD}%(objectname){FIELD}%(contents:subject){FIELD}%(committerdate:iso-strict){FIELD}%(worktreepath)"
    );
    let output = text(&run(
        root,
        [
            "for-each-ref",
            "--sort=-committerdate",
            format.as_str(),
            "refs/heads",
        ],
    )?);
    let here = status(root).ok().and_then(|status| status.branch);
    Ok(output
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split(FIELD).collect();
            if fields.len() < 7 {
                return None;
            }
            let current = fields[1] == "*";
            Some(BranchInfo {
                name: fields[0].to_owned(),
                current,
                upstream: (!fields[2].is_empty()).then(|| fields[2].to_owned()),
                oid: fields[3].to_owned(),
                subject: fields[4].to_owned(),
                committed_at: fields[5].to_owned(),
                checked_out_elsewhere: !current
                    && !fields[6].is_empty()
                    && here.as_deref() != Some(fields[0]),
            })
        })
        .collect())
}

fn valid_branch_name(root: &Path, name: &str) -> Result<(), ScmError> {
    if name.starts_with('-') || run(root, ["check-ref-format", "--branch", name]).is_err() {
        return Err(ScmError::Refused(format!(
            "`{name}` is not a valid branch name"
        )));
    }
    Ok(())
}

/// Switch the working directory to `branch`. Git itself refuses when local
/// changes would be overwritten, and that refusal is passed on as it is.
pub fn switch(root: &Path, branch: &str) -> Result<(), ScmError> {
    valid_branch_name(root, branch)?;
    run(root, ["switch", "--", branch])?;
    Ok(())
}

/// Create `name` at `start` (HEAD when absent) and switch to it.
pub fn create_branch(root: &Path, name: &str, start: Option<&str>) -> Result<(), ScmError> {
    valid_branch_name(root, name)?;
    let mut arguments = vec!["switch", "-c", name];
    if let Some(start) = start {
        if start.starts_with('-') {
            return Err(ScmError::Refused("not a revision".into()));
        }
        arguments.push(start);
    }
    run(root, arguments)?;
    Ok(())
}

/// Fetch every remote, pruning branches deleted upstream.
pub fn fetch(root: &Path) -> Result<String, ScmError> {
    let output = run(root, ["fetch", "--all", "--prune"])?;
    Ok(String::from_utf8_lossy(&output.stderr).trim().to_owned())
}

/// Bring the current branch up to its upstream, only if that needs no merge.
pub fn pull(root: &Path) -> Result<String, ScmError> {
    let output = run(root, ["pull", "--ff-only"])?;
    Ok(text(&output).trim().to_owned())
}

/// Push `branch` to `remote`, setting it as the upstream when it has none.
/// Publishing is outside this machine, so the screen confirms the exact
/// remote and branch first (ADR-012).
pub fn push(root: &Path, remote: &str, branch: &str) -> Result<String, ScmError> {
    if remote.starts_with('-') || remote.is_empty() {
        return Err(ScmError::Refused(format!("`{remote}` is not a remote")));
    }
    valid_branch_name(root, branch)?;
    let output = run(
        root,
        [
            "push",
            "--set-upstream",
            remote,
            &format!("refs/heads/{branch}:refs/heads/{branch}"),
        ],
    )?;
    Ok(String::from_utf8_lossy(&output.stderr).trim().to_owned())
}

/// The remotes this repository knows, by name.
pub fn remotes(root: &Path) -> Result<Vec<String>, ScmError> {
    Ok(text(&run(root, ["remote"])?)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

/// Set the working directory's changes aside, untracked files included.
pub fn stash_push(root: &Path, message: &str) -> Result<(), ScmError> {
    let mut arguments = vec!["stash", "push", "--include-untracked"];
    if !message.trim().is_empty() {
        arguments.extend(["-m", message.trim()]);
    }
    run(root, arguments)?;
    Ok(())
}

/// Bring back the most recently stashed changes.
pub fn stash_pop(root: &Path) -> Result<(), ScmError> {
    run(root, ["stash", "pop"])?;
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StashEntry {
    pub reference: String,
    pub message: String,
}

pub fn stashes(root: &Path) -> Result<Vec<StashEntry>, ScmError> {
    let format = format!("--format=%gd{FIELD}%gs");
    Ok(text(&run(root, ["stash", "list", format.as_str()])?)
        .lines()
        .filter_map(|line| {
            let (reference, message) = line.split_once(FIELD)?;
            Some(StashEntry {
                reference: reference.to_owned(),
                message: message.to_owned(),
            })
        })
        .collect())
}

// --- integration ---------------------------------------------------------

/// What merging a task's branch into its base would do, worked out without
/// touching either.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationPreview {
    /// The repository the merge happens in: where the base is checked out.
    pub root: PathBuf,
    pub base: String,
    pub base_oid: String,
    pub head: String,
    pub head_oid: String,
    /// Commits on the head branch that the base does not have, oldest last.
    pub commits: Vec<CommitInfo>,
    /// `git diff --stat` between the base and the head.
    pub files_changed: Vec<String>,
    /// Files that would conflict, from `git merge-tree`. Empty means it would
    /// merge cleanly.
    pub conflicts: Vec<String>,
    /// The head contains the base, so the base could simply move forward.
    pub fast_forward: bool,
    /// Why it cannot be applied as things stand, if anything.
    pub blockers: Vec<String>,
    /// Over what the merge would do. Applying refuses any other.
    pub fingerprint: String,
}

/// How a task's branch is brought into its base.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationStrategy {
    /// A merge commit, keeping the branch's commits as they are.
    Merge,
    /// One commit on the base holding the whole change.
    Squash,
    /// Move the base forward to the head; refused unless that is possible.
    FastForward,
}

/// Preview merging `head` into `base` in the repository at `root`.
pub fn preview_integration(
    root: &Path,
    base: &str,
    head: &str,
) -> Result<IntegrationPreview, ScmError> {
    valid_branch_name(root, base)?;
    valid_branch_name(root, head)?;
    let base_oid = rev(root, base)?;
    let head_oid = rev(root, head)?;
    let commits = log(root, Some(&format!("{base_oid}..{head_oid}")), 200, 0)?;
    let files_changed = text(&run(
        root,
        [
            "diff",
            "--no-ext-diff",
            "--name-status",
            &format!("{base_oid}...{head_oid}"),
        ],
    )?)
    .lines()
    .map(|line| line.replace('\t', "  "))
    .collect();
    let fast_forward = run(root, ["merge-base", "--is-ancestor", &base_oid, &head_oid]).is_ok();
    let conflicts = merge_tree_conflicts(root, &base_oid, &head_oid)?;

    let mut blockers = Vec::new();
    let here = status(root)?;
    if here.branch.as_deref() != Some(base) {
        blockers.push(format!(
            "{} has {} checked out, not {base}; switch to {base} there first",
            root.display(),
            here.branch.as_deref().unwrap_or("a detached HEAD")
        ));
    }
    if here.changes.iter().any(|change| !change.untracked) {
        blockers.push(format!(
            "{} has uncommitted changes; commit or stash them first",
            root.display()
        ));
    }
    if let Some(operation) = &here.operation {
        blockers.push(format!("a {operation} is already in progress there"));
    }
    if commits.is_empty() {
        blockers.push(format!(
            "{head} has nothing that {base} does not already have"
        ));
    }

    let fingerprint = digest(&[
        root.to_string_lossy().as_ref(),
        base,
        &base_oid,
        head,
        &head_oid,
    ]);
    Ok(IntegrationPreview {
        root: root.to_owned(),
        base: base.to_owned(),
        base_oid,
        head: head.to_owned(),
        head_oid,
        commits,
        files_changed,
        conflicts,
        fast_forward,
        blockers,
        fingerprint,
    })
}

/// Merge `head` into `base` as previewed. Refuses a preview that no longer
/// matches, anything the preview listed as blocking, and a fast-forward that
/// is not possible. On conflict the merge is aborted and the conflicting files
/// are reported: the base is left exactly as it was.
pub fn apply_integration(
    root: &Path,
    base: &str,
    head: &str,
    fingerprint: &str,
    strategy: IntegrationStrategy,
    message: &str,
) -> Result<CommitInfo, ScmError> {
    let preview = preview_integration(root, base, head)?;
    if preview.fingerprint != fingerprint {
        return Err(ScmError::PreviewChanged {
            approved: fingerprint.to_owned(),
            current: preview.fingerprint,
        });
    }
    if let Some(blocker) = preview.blockers.first() {
        return Err(ScmError::Refused(blocker.clone()));
    }
    let message = if message.trim().is_empty() {
        format!("Merge {head} into {base}")
    } else {
        message.trim().to_owned()
    };
    let result = match strategy {
        IntegrationStrategy::FastForward => {
            if !preview.fast_forward {
                return Err(ScmError::Refused(format!(
                    "{base} has moved since {head} branched; it cannot simply move forward"
                )));
            }
            run(root, ["merge", "--ff-only", &preview.head_oid]).map(|_| ())
        }
        IntegrationStrategy::Merge => run(
            root,
            [
                "merge",
                "--no-ff",
                "--no-edit",
                "-m",
                &message,
                &preview.head_oid,
            ],
        )
        .map(|_| ()),
        IntegrationStrategy::Squash => run(root, ["merge", "--squash", &preview.head_oid])
            .and_then(|_| run(root, ["commit", "-m", &message]).map(|_| ())),
    };
    if result.is_err() {
        let conflicts: Vec<String> = status(root)?
            .changes
            .into_iter()
            .filter(|change| change.conflicted)
            .map(|change| change.path)
            .collect();
        // Leave the base as it was. A squash leaves no MERGE_HEAD, so it is
        // reset to the commit it started from instead.
        let _ = run(root, ["merge", "--abort"]);
        if strategy == IntegrationStrategy::Squash {
            let _ = run(root, ["reset", "--hard", "-q", &preview.base_oid]);
        }
        if !conflicts.is_empty() {
            return Err(ScmError::Conflicts {
                base: base.to_owned(),
                head: head.to_owned(),
                files: conflicts,
            });
        }
        result?;
    }
    log(root, None, 1, 0)?
        .into_iter()
        .next()
        .ok_or_else(|| ScmError::Refused("the merge could not be read back".into()))
}

fn rev(root: &Path, name: &str) -> Result<String, ScmError> {
    Ok(run(
        root,
        [
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{name}^{{commit}}"),
        ],
    )?
    .stdout_text())
}

/// Files a merge of `head` into `base` would conflict on, worked out in
/// Git's object store without touching any working tree.
fn merge_tree_conflicts(root: &Path, base: &str, head: &str) -> Result<Vec<String>, ScmError> {
    let output = mars_core::process::command("git")
        .arg("-C")
        .arg(root)
        .args([
            "merge-tree",
            "--write-tree",
            "--name-only",
            "--no-messages",
            base,
            head,
        ])
        .output()
        .map_err(|source| GitCommandError::Launch {
            cwd: root.display().to_string(),
            source,
        })?;
    match output.status.code() {
        // Clean: one line, the tree id.
        Some(0) => Ok(Vec::new()),
        // Conflicts: the tree id, then one conflicted path per line.
        Some(1) => Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .skip(1)
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect()),
        code => Err(ScmError::Git(GitCommandError::Failed {
            exit_code: code,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })),
    }
}

fn digest(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The commit a task's change is measured from: where its branch left `base`.
pub fn merge_base(root: &Path, base: &str) -> Result<String, ScmError> {
    Ok(run(root, ["merge-base", "HEAD", base])?.stdout_text())
}

/// Everything a task changed since it left its base — committed or not — as
/// one unified diff, with the untracked files it added listed separately
/// because a diff cannot mention a file Git has never seen.
pub fn change_since(root: &Path, base: &str) -> Result<(String, Vec<String>), ScmError> {
    let from = merge_base(root, base)?;
    let diff = text(&run(root, ["diff", "--no-ext-diff", "--unified=3", &from])?);
    let untracked = text(&run(root, ["ls-files", "--others", "--exclude-standard"])?)
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    Ok((diff, untracked))
}
