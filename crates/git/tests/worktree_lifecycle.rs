mod fixtures;

use std::fs;

use mars_git::{TaskWorktreeError, TaskWorktreeService, WorktreeState};

use fixtures::{GitFixture, git};

#[test]
fn plans_resolve_same_title_collisions_without_mutating_git() {
    let fixture = GitFixture::new();
    let service = TaskWorktreeService::new(fixture.storage, &fixture.worktrees_root);

    let first = service
        .create_task(&fixture.repository_id, "Add auth flow", "first", "HEAD")
        .expect("first task should plan");
    let second = service
        .create_task(&fixture.repository_id, "Add auth flow", "second", "HEAD")
        .expect("second task should plan");

    assert_eq!(first.branch_name, "mars/add-auth-flow");
    assert_eq!(second.branch_name, "mars/add-auth-flow-2");
    assert!(first.worktree_path.ends_with("add-auth-flow"));
    assert!(second.worktree_path.ends_with("add-auth-flow-2"));
    assert!(!first.worktree_path.exists());
    assert!(!second.worktree_path.exists());
    assert!(!git(&fixture.repository_root, &["branch", "--list", "mars/*"]).contains("mars/"));
}

#[test]
fn planning_avoids_matching_branches_that_already_exist_in_git() {
    let fixture = GitFixture::new();
    git(
        &fixture.repository_root,
        &["branch", "mars/add-auth-flow", "HEAD"],
    );
    let service = TaskWorktreeService::new(fixture.storage, &fixture.worktrees_root);

    let plan = service
        .create_task(&fixture.repository_id, "Add auth flow", "new work", "HEAD")
        .expect("task should plan around an existing branch");

    assert_eq!(plan.branch_name, "mars/add-auth-flow-2");
    assert!(plan.worktree_path.ends_with("add-auth-flow-2"));
}

#[test]
fn provisions_and_removes_a_clean_worktree_with_paths_containing_spaces() {
    let fixture = GitFixture::new();
    let service = TaskWorktreeService::new(fixture.storage, &fixture.worktrees_root);
    let plan = service
        .create_task(
            &fixture.repository_id,
            "Safe change",
            "implement it",
            "HEAD",
        )
        .expect("task should plan");

    let ready = service
        .provision(&plan.task_id)
        .expect("worktree should provision");
    assert_eq!(ready.state, WorktreeState::Ready);
    assert!(ready.worktree_path.join("README.md").is_file());
    assert!(git(&ready.worktree_path, &["branch", "--show-current"]).contains("mars/safe-change"));

    let removed = service
        .remove(&plan.task_id)
        .expect("clean worktree should remove");
    assert_eq!(removed.state, WorktreeState::Removed);
    assert!(!removed.worktree_path.exists());
    assert!(
        !git(
            &fixture.repository_root,
            &["branch", "--list", "mars/safe-change"]
        )
        .contains("mars/safe-change")
    );
    assert!(
        service
            .mutation_journal(&plan.task_id)
            .expect("journal should load")
            .iter()
            .all(|entry| entry.status == "completed")
    );
}

#[test]
fn refuses_dirty_removal_and_reconciles_a_missing_worktree() {
    let fixture = GitFixture::new();
    let service = TaskWorktreeService::new(fixture.storage, &fixture.worktrees_root);
    let plan = service
        .create_task(
            &fixture.repository_id,
            "Keep changes",
            "protect edits",
            "HEAD",
        )
        .expect("task should plan");
    let ready = service
        .provision(&plan.task_id)
        .expect("worktree should provision");
    fs::write(ready.worktree_path.join("README.md"), "dirty\n").expect("edit should write");

    assert!(matches!(
        service.remove(&plan.task_id),
        Err(TaskWorktreeError::DirtyWorktree { .. })
    ));
    assert_eq!(
        service.get(&plan.task_id).expect("plan should load").state,
        WorktreeState::Ready
    );

    fs::remove_dir_all(&ready.worktree_path).expect("fixture simulates external removal");
    let reconciled = service
        .reconcile(&plan.task_id)
        .expect("state should reconcile");
    assert_eq!(reconciled.state, WorktreeState::Orphaned);
}

#[test]
fn invalid_preflight_and_failed_creation_leave_no_branch_or_worktree() {
    let fixture = GitFixture::new();
    let service = TaskWorktreeService::new(fixture.storage.clone(), &fixture.worktrees_root);
    let invalid = service
        .create_task(
            &fixture.repository_id,
            "Bad base",
            "fail early",
            "missing-ref",
        )
        .expect("task should plan");
    assert!(matches!(
        service.provision(&invalid.task_id),
        Err(TaskWorktreeError::InvalidBaseRef { .. })
    ));
    assert!(!invalid.worktree_path.exists());

    let blocked_root = fixture.worktrees_root.join("not-a-directory");
    fs::write(&blocked_root, "block worktree creation").expect("blocker should write");
    let failing_service = TaskWorktreeService::new(fixture.storage, &blocked_root);
    let failing = failing_service
        .create_task(
            &fixture.repository_id,
            "Rollback branch",
            "force add failure",
            "HEAD",
        )
        .expect("task should plan");
    assert!(failing_service.provision(&failing.task_id).is_err());
    assert!(
        !git(
            &fixture.repository_root,
            &["branch", "--list", &failing.branch_name]
        )
        .contains(&failing.branch_name)
    );
    assert!(!failing.worktree_path.exists());
    assert!(
        failing_service
            .mutation_journal(&failing.task_id)
            .expect("journal should load")
            .iter()
            .any(|entry| entry.status == "rolled_back")
    );
}
