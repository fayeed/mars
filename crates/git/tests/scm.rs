//! Source control over real repositories: status, staging, committing,
//! history, branches, remotes, and bringing a task's branch home.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mars_git::scm::{self, IntegrationStrategy, ScmError};

struct Repo {
    root: PathBuf,
    _directory: TestDirectory,
}

impl Repo {
    fn new() -> Self {
        let directory = TestDirectory::new();
        let root = directory.path().join("repo");
        fs::create_dir_all(&root).expect("repo dir");
        git(&root, &["init", "-q", "-b", "main"]);
        git(&root, &["config", "user.name", "Mars Test"]);
        git(&root, &["config", "user.email", "mars@example.test"]);
        git(&root, &["config", "commit.gpgsign", "false"]);
        // Byte-for-byte assertions should not depend on this machine's global
        // line-ending conversion.
        git(&root, &["config", "core.autocrlf", "false"]);
        fs::write(root.join("README.md"), "hello\n").expect("write");
        fs::write(root.join("app.txt"), "one\ntwo\nthree\n").expect("write");
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-q", "-m", "initial"]);
        Self {
            root,
            _directory: directory,
        }
    }

    fn write(&self, path: &str, contents: &str) {
        let path = self.root.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent");
        }
        fs::write(path, contents).expect("write");
    }

    fn head(&self) -> String {
        git(&self.root, &["rev-parse", "HEAD"])
    }
}

fn git(root: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

#[test]
fn status_reads_staged_unstaged_untracked_and_renamed_files() {
    let repo = Repo::new();
    repo.write("app.txt", "one\nTWO\nthree\n");
    repo.write("notes/new.md", "new\n");
    git(&repo.root, &["mv", "README.md", "GUIDE.md"]);
    repo.write("staged.txt", "staged\n");
    git(&repo.root, &["add", "staged.txt"]);

    let status = scm::status(&repo.root).expect("status");
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert_eq!(status.head_oid.as_deref(), Some(repo.head().as_str()));
    let find = |path: &str| {
        status
            .changes
            .iter()
            .find(|change| change.path == path)
            .unwrap_or_else(|| panic!("{path} in {:?}", status.changes))
    };
    assert_eq!(find("app.txt").unstaged, 'M');
    assert_eq!(find("app.txt").staged, '.');
    assert!(find("notes/new.md").untracked);
    assert_eq!(find("staged.txt").staged, 'A');
    let renamed = find("GUIDE.md");
    assert_eq!(renamed.staged, 'R');
    assert_eq!(renamed.original_path.as_deref(), Some("README.md"));
    assert!(!status.is_clean());
}

#[test]
fn staging_is_exactly_what_was_named_and_commit_needs_something_staged() {
    let repo = Repo::new();
    repo.write("app.txt", "changed\n");
    repo.write("other.txt", "other\n");

    assert!(matches!(
        scm::commit(&repo.root, "nothing staged", false),
        Err(ScmError::Refused(_))
    ));
    assert!(matches!(
        scm::commit(&repo.root, "   ", false),
        Err(ScmError::Refused(_))
    ));

    scm::stage(&repo.root, &["app.txt".into()]).expect("stage");
    let status = scm::status(&repo.root).expect("status");
    assert!(
        status
            .changes
            .iter()
            .any(|change| change.path == "app.txt" && change.staged == 'M')
    );
    assert!(
        status
            .changes
            .iter()
            .any(|change| change.path == "other.txt" && change.untracked)
    );

    let staged_diff = scm::diff(&repo.root, Some("app.txt"), true).expect("diff");
    assert!(staged_diff.contains("+changed"));
    let untracked_diff = scm::diff(&repo.root, Some("other.txt"), false).expect("diff");
    assert!(untracked_diff.contains("+other"), "{untracked_diff}");

    let commit = scm::commit(&repo.root, "Change the app", false).expect("commit");
    assert_eq!(commit.subject, "Change the app");
    let status = scm::status(&repo.root).expect("status");
    assert!(
        status.changes.iter().all(|change| change.path != "app.txt"),
        "the staged file was committed"
    );
    assert!(
        status
            .changes
            .iter()
            .any(|change| change.path == "other.txt"),
        "the unstaged file was left alone"
    );

    let log = scm::log(&repo.root, None, 10, 0).expect("log");
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].subject, "Change the app");
    assert!(log[0].refs.contains(&"main".to_owned()));
    let shown = scm::show(&repo.root, &log[0].oid).expect("show");
    assert!(shown.contains("+changed"));
    assert!(matches!(
        scm::show(&repo.root, "--help"),
        Err(ScmError::Refused(_))
    ));
}

#[test]
fn unstage_and_discard_put_files_back() {
    let repo = Repo::new();
    repo.write("app.txt", "changed\n");
    repo.write("scratch.txt", "throwaway\n");
    scm::stage(&repo.root, &["app.txt".into()]).expect("stage");
    scm::unstage(&repo.root, &["app.txt".into()]).expect("unstage");
    let status = scm::status(&repo.root).expect("status");
    let app = status
        .changes
        .iter()
        .find(|change| change.path == "app.txt")
        .expect("app.txt changed");
    assert_eq!((app.staged, app.unstaged), ('.', 'M'));

    scm::discard(&repo.root, &["app.txt".into(), "scratch.txt".into()]).expect("discard");
    assert_eq!(
        fs::read_to_string(repo.root.join("app.txt")).expect("read"),
        "one\ntwo\nthree\n"
    );
    assert!(!repo.root.join("scratch.txt").exists());
    assert!(scm::status(&repo.root).expect("status").is_clean());

    assert!(matches!(
        scm::stage(&repo.root, &["--all".into()]),
        Err(ScmError::Refused(_))
    ));
    assert!(matches!(
        scm::stage(&repo.root, &[]),
        Err(ScmError::Refused(_))
    ));
}

#[test]
fn branches_list_switch_and_refuse_bad_names() {
    let repo = Repo::new();
    scm::create_branch(&repo.root, "feature/one", None).expect("create");
    assert_eq!(
        scm::status(&repo.root).expect("status").branch.as_deref(),
        Some("feature/one")
    );
    let branches = scm::branches(&repo.root).expect("branches");
    let names: Vec<&str> = branches.iter().map(|branch| branch.name.as_str()).collect();
    assert!(names.contains(&"main") && names.contains(&"feature/one"));
    assert!(
        branches
            .iter()
            .any(|branch| branch.name == "feature/one" && branch.current)
    );

    scm::switch(&repo.root, "main").expect("switch");
    assert_eq!(
        scm::status(&repo.root).expect("status").branch.as_deref(),
        Some("main")
    );
    assert!(matches!(
        scm::create_branch(&repo.root, "bad name", None),
        Err(ScmError::Refused(_))
    ));
    assert!(matches!(
        scm::switch(&repo.root, "-f"),
        Err(ScmError::Refused(_))
    ));
}

#[test]
fn push_fetch_and_pull_talk_to_a_real_remote() {
    let repo = Repo::new();
    let remote = repo.root.parent().expect("parent").join("remote.git");
    git(
        repo.root.parent().expect("parent"),
        &[
            "init",
            "-q",
            "--bare",
            "-b",
            "main",
            remote.to_str().expect("utf-8"),
        ],
    );
    git(
        &repo.root,
        &["remote", "add", "origin", remote.to_str().expect("utf-8")],
    );
    assert_eq!(
        scm::remotes(&repo.root).expect("remotes"),
        vec!["origin".to_owned()]
    );

    scm::push(&repo.root, "origin", "main").expect("push");
    let status = scm::status(&repo.root).expect("status");
    assert_eq!(status.upstream.as_deref(), Some("origin/main"));
    assert_eq!((status.ahead, status.behind), (0, 0));

    // A second clone moves the remote forward; the first pulls it.
    let other = repo.root.parent().expect("parent").join("other");
    git(
        repo.root.parent().expect("parent"),
        &[
            "clone",
            "-q",
            "-c",
            "core.autocrlf=false",
            remote.to_str().expect("utf-8"),
            other.to_str().expect("utf-8"),
        ],
    );
    git(&other, &["config", "user.name", "Other"]);
    git(&other, &["config", "user.email", "other@example.test"]);
    fs::write(other.join("app.txt"), "remote change\n").expect("write");
    git(&other, &["commit", "-qam", "remote change"]);
    git(&other, &["push", "-q", "origin", "main"]);

    scm::fetch(&repo.root).expect("fetch");
    let status = scm::status(&repo.root).expect("status");
    assert_eq!(status.behind, 1);
    scm::pull(&repo.root).expect("pull");
    assert_eq!(
        fs::read_to_string(repo.root.join("app.txt")).expect("read"),
        "remote change\n"
    );
    assert!(matches!(
        scm::push(&repo.root, "--mirror", "main"),
        Err(ScmError::Refused(_))
    ));
}

#[test]
fn stash_sets_work_aside_and_brings_it_back() {
    let repo = Repo::new();
    repo.write("app.txt", "work in progress\n");
    repo.write("new.txt", "new\n");
    scm::stash_push(&repo.root, "before switching").expect("stash");
    assert!(scm::status(&repo.root).expect("status").is_clean());
    let stashes = scm::stashes(&repo.root).expect("stashes");
    assert_eq!(stashes.len(), 1);
    assert!(stashes[0].message.contains("before switching"));
    scm::stash_pop(&repo.root).expect("pop");
    assert!(repo.root.join("new.txt").is_file());
    assert_eq!(
        fs::read_to_string(repo.root.join("app.txt")).expect("read"),
        "work in progress\n"
    );
}

#[test]
fn a_clean_task_branch_merges_home_as_previewed() {
    let repo = Repo::new();
    git(&repo.root, &["branch", "task/one"]);
    // The task's work, made as an agent would make it: in its own worktree.
    let worktree = repo.root.parent().expect("parent").join("task-one");
    git(
        &repo.root,
        &[
            "worktree",
            "add",
            "-q",
            worktree.to_str().expect("utf-8"),
            "task/one",
        ],
    );
    fs::write(worktree.join("feature.txt"), "feature\n").expect("write");
    git(&worktree, &["add", "feature.txt"]);
    git(&worktree, &["commit", "-qm", "Add the feature"]);

    let (diff, untracked) = scm::change_since(&worktree, "main").expect("change since base");
    assert!(
        diff.contains("+feature"),
        "committed work is part of the task's change"
    );
    assert!(untracked.is_empty());

    let preview = scm::preview_integration(&repo.root, "main", "task/one").expect("preview");
    assert!(preview.blockers.is_empty(), "{:?}", preview.blockers);
    assert!(preview.conflicts.is_empty());
    assert!(preview.fast_forward);
    assert_eq!(preview.commits.len(), 1);
    assert!(
        preview
            .files_changed
            .iter()
            .any(|line| line.contains("feature.txt"))
    );

    let merged = scm::apply_integration(
        &repo.root,
        "main",
        "task/one",
        &preview.fingerprint,
        IntegrationStrategy::Merge,
        "",
    )
    .expect("merge");
    assert_eq!(merged.subject, "Merge task/one into main");
    assert!(repo.root.join("feature.txt").is_file());
}

#[test]
fn a_squash_lands_as_one_commit() {
    let repo = Repo::new();
    git(&repo.root, &["switch", "-q", "-c", "task/squash"]);
    for index in 0..3 {
        repo.write("feature.txt", &format!("step {index}\n"));
        git(&repo.root, &["add", "feature.txt"]);
        git(&repo.root, &["commit", "-qm", &format!("step {index}")]);
    }
    git(&repo.root, &["switch", "-q", "main"]);
    let preview = scm::preview_integration(&repo.root, "main", "task/squash").expect("preview");
    assert_eq!(preview.commits.len(), 3);
    let squashed = scm::apply_integration(
        &repo.root,
        "main",
        "task/squash",
        &preview.fingerprint,
        IntegrationStrategy::Squash,
        "Add the feature",
    )
    .expect("squash");
    assert_eq!(squashed.subject, "Add the feature");
    assert_eq!(scm::log(&repo.root, None, 10, 0).expect("log").len(), 2);
}

#[test]
fn a_conflicting_merge_is_predicted_stops_and_leaves_the_base_as_it_was() {
    let repo = Repo::new();
    git(&repo.root, &["switch", "-q", "-c", "task/conflict"]);
    repo.write("app.txt", "task side\n");
    git(&repo.root, &["commit", "-qam", "task edit"]);
    git(&repo.root, &["switch", "-q", "main"]);
    repo.write("app.txt", "base side\n");
    git(&repo.root, &["commit", "-qam", "base edit"]);
    let before = repo.head();

    let preview = scm::preview_integration(&repo.root, "main", "task/conflict").expect("preview");
    assert_eq!(preview.conflicts, vec!["app.txt".to_owned()]);
    assert!(!preview.fast_forward);

    let outcome = scm::apply_integration(
        &repo.root,
        "main",
        "task/conflict",
        &preview.fingerprint,
        IntegrationStrategy::Merge,
        "",
    );
    match outcome {
        Err(ScmError::Conflicts { files, .. }) => assert_eq!(files, vec!["app.txt".to_owned()]),
        other => panic!("expected conflicts, got {other:?}"),
    }
    assert_eq!(repo.head(), before, "the base did not move");
    let status = scm::status(&repo.root).expect("status");
    assert!(status.is_clean(), "{:?}", status.changes);
    assert_eq!(status.operation, None, "no merge left half done");

    // The same, squashed.
    let outcome = scm::apply_integration(
        &repo.root,
        "main",
        "task/conflict",
        &preview.fingerprint,
        IntegrationStrategy::Squash,
        "squash it",
    );
    assert!(matches!(outcome, Err(ScmError::Conflicts { .. })));
    assert_eq!(repo.head(), before);
    assert!(scm::status(&repo.root).expect("status").is_clean());
}

#[test]
fn integration_refuses_a_stale_preview_a_dirty_base_and_an_impossible_fast_forward() {
    let repo = Repo::new();
    git(&repo.root, &["switch", "-q", "-c", "task/stale"]);
    repo.write("feature.txt", "feature\n");
    git(&repo.root, &["add", "feature.txt"]);
    git(&repo.root, &["commit", "-qm", "feature"]);
    git(&repo.root, &["switch", "-q", "main"]);
    let preview = scm::preview_integration(&repo.root, "main", "task/stale").expect("preview");

    // The task moved after the preview was approved.
    git(&repo.root, &["switch", "-q", "task/stale"]);
    repo.write("feature.txt", "more\n");
    git(&repo.root, &["commit", "-qam", "more"]);
    git(&repo.root, &["switch", "-q", "main"]);
    assert!(matches!(
        scm::apply_integration(
            &repo.root,
            "main",
            "task/stale",
            &preview.fingerprint,
            IntegrationStrategy::Merge,
            ""
        ),
        Err(ScmError::PreviewChanged { .. })
    ));

    repo.write("app.txt", "uncommitted\n");
    let dirty = scm::preview_integration(&repo.root, "main", "task/stale").expect("preview");
    assert!(
        dirty
            .blockers
            .iter()
            .any(|blocker| blocker.contains("uncommitted"))
    );
    assert!(matches!(
        scm::apply_integration(
            &repo.root,
            "main",
            "task/stale",
            &dirty.fingerprint,
            IntegrationStrategy::Merge,
            ""
        ),
        Err(ScmError::Refused(_))
    ));
    git(&repo.root, &["checkout", "-q", "--", "app.txt"]);

    repo.write("base.txt", "base moved\n");
    git(&repo.root, &["add", "base.txt"]);
    git(&repo.root, &["commit", "-qm", "base moved"]);
    let moved = scm::preview_integration(&repo.root, "main", "task/stale").expect("preview");
    assert!(!moved.fast_forward);
    assert!(matches!(
        scm::apply_integration(
            &repo.root,
            "main",
            "task/stale",
            &moved.fingerprint,
            IntegrationStrategy::FastForward,
            ""
        ),
        Err(ScmError::Refused(_))
    ));
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("mars-scm-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).expect("temp");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        for _ in 0..50 {
            if fs::remove_dir_all(&self.0).is_ok() || !self.0.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
