mod fixtures;

use std::fs;
use std::sync::Arc;

use mars_activity::{ActivityKind, ActivityRecord};
use mars_git::{
    DriftChoice, PlanRequest, PreflightCode, PreflightSeverity, SetupCommand, WorktreePlanner,
    WorktreeState,
};

use fixtures::{GitFixture, git};

fn planner(fixture: &GitFixture) -> (WorktreePlanner, ActivityRecord) {
    let payload_root = fixture.worktrees_root.join("activity-payloads");
    let activity = ActivityRecord::new(Arc::clone(&fixture.storage), &payload_root);
    let planner =
        WorktreePlanner::new(Arc::clone(&fixture.storage), &fixture.worktrees_root).recording_to(
            ActivityRecord::new(Arc::clone(&fixture.storage), &payload_root),
        );
    (planner, activity)
}

fn request(fixture: &GitFixture, title: &str) -> PlanRequest {
    PlanRequest {
        repository_id: fixture.repository_id.clone(),
        title: title.to_owned(),
        intent: "implement it".into(),
        base_ref: "HEAD".into(),
        expected_scope: vec!["src/**".into()],
        setup_commands: Vec::new(),
    }
}

#[test]
fn a_plan_names_everything_it_will_do_and_mutates_nothing() {
    let fixture = GitFixture::new();
    let (planner, activity) = planner(&fixture);

    let proposal = planner
        .propose(PlanRequest {
            expected_scope: vec!["src/**".into(), "tests/**".into()],
            setup_commands: vec![SetupCommand::new("npm ci", "install dependencies")],
            ..request(&fixture, "Add auth flow")
        })
        .expect("plan should build");

    assert_eq!(proposal.branch_name, "mars/add-auth-flow");
    assert_eq!(proposal.base_ref, "HEAD");
    assert_eq!(proposal.base_oid.len(), 40, "the plan pins a real commit");
    assert_eq!(proposal.expected_scope, vec!["src/**", "tests/**"]);
    assert_eq!(proposal.setup_commands[0].classification, "network");
    assert_eq!(proposal.state, WorktreeState::Planned);
    assert!(!proposal.fingerprint.is_empty());
    assert!(!proposal.is_approved());
    assert!(!proposal.is_blocked());

    assert!(
        !proposal.worktree_path.exists(),
        "planning creates no directory"
    );
    assert!(
        !git(&fixture.repository_root, &["branch", "--list", "mars/*"]).contains("mars/"),
        "planning creates no branch"
    );

    let planned = activity
        .read(&proposal.task_id, 0, 10)
        .expect("log should read");
    assert_eq!(planned.len(), 1);
    assert_eq!(planned[0].kind, ActivityKind::WorktreePlanned);
    assert_eq!(planned[0].payload["baseOid"], proposal.base_oid.as_str());
}

#[test]
fn the_plan_reports_free_space_on_the_volume_it_would_use() {
    let fixture = GitFixture::new();
    let (planner, _) = planner(&fixture);

    let proposal = planner
        .propose(request(&fixture, "Disk aware"))
        .expect("plan should build");

    assert!(
        proposal.disk.available_bytes.is_some(),
        "the planner should be able to read free space on a real volume"
    );
    assert_eq!(proposal.disk.threshold_bytes, 2 * 1024 * 1024 * 1024);
}

#[test]
fn a_dirty_base_is_a_warning_that_says_what_happens_to_the_uncommitted_work() {
    let fixture = GitFixture::new();
    fs::write(fixture.repository_root.join("README.md"), "# edited\n")
        .expect("base should be dirtied");
    let (planner, _) = planner(&fixture);

    let proposal = planner
        .propose(request(&fixture, "Dirty base"))
        .expect("plan should build over a dirty base");

    let finding = proposal
        .preflight
        .iter()
        .find(|finding| finding.code == PreflightCode::DirtyBase)
        .expect("a dirty base should be reported");
    assert_eq!(finding.severity, PreflightSeverity::Warning);
    assert!(!proposal.is_blocked(), "a dirty base does not block");
    assert_eq!(finding.resolutions.len(), 2);
    assert!(
        finding.resolutions[0]
            .consequence
            .contains("uncommitted changes stay where they are")
    );
}

#[test]
fn an_occupied_path_blocks_the_plan_and_offers_a_way_out() {
    let fixture = GitFixture::new();
    let (planner, _) = planner(&fixture);
    let first = planner
        .propose(request(&fixture, "Occupied"))
        .expect("first plan should build");
    fs::create_dir_all(&first.worktree_path).expect("path should be occupied");

    let refreshed = planner
        .current(&first.task_id)
        .expect("plan should refresh against the world as it is");

    assert!(refreshed.is_blocked());
    let finding = refreshed
        .preflight
        .iter()
        .find(|finding| finding.code == PreflightCode::WorktreeExists)
        .expect("an occupied path should be reported");
    assert_eq!(finding.severity, PreflightSeverity::Blocking);
    assert!(
        finding
            .resolutions
            .iter()
            .any(|resolution| resolution.consequence.contains("Mars will not delete it"))
    );
    assert!(
        planner
            .approve(&first.task_id, &refreshed.fingerprint, "fayeed")
            .is_err(),
        "a blocked plan must not be approvable"
    );
}

#[test]
fn an_existing_branch_blocks_a_plan_that_would_reuse_its_name() {
    let fixture = GitFixture::new();
    let (planner, _) = planner(&fixture);
    let proposal = planner
        .propose(request(&fixture, "Branch clash"))
        .expect("plan should build");
    git(
        &fixture.repository_root,
        &["branch", &proposal.branch_name, "HEAD"],
    );

    let refreshed = planner
        .current(&proposal.task_id)
        .expect("plan should refresh");

    assert!(refreshed.is_blocked());
    assert!(
        refreshed
            .preflight
            .iter()
            .any(|finding| finding.code == PreflightCode::BranchExists)
    );
}

#[test]
fn approval_is_recorded_against_the_exact_plan_that_was_shown() {
    let fixture = GitFixture::new();
    let (planner, _) = planner(&fixture);
    let proposal = planner
        .propose(request(&fixture, "Exact approval"))
        .expect("plan should build");

    assert!(
        planner
            .approve(&proposal.task_id, "not-the-fingerprint", "fayeed")
            .is_err(),
        "an approval against a different plan must be refused"
    );

    let approved = planner
        .approve(&proposal.task_id, &proposal.fingerprint, "fayeed")
        .expect("the exact plan should approve");
    assert_eq!(approved.approved_by.as_deref(), Some("fayeed"));
    assert!(approved.approved_at.is_some());
}

#[test]
fn provisioning_refuses_when_the_base_moved_after_approval() {
    let fixture = GitFixture::new();
    let (planner, _) = planner(&fixture);
    let proposal = planner
        .propose(request(&fixture, "Changed under us"))
        .expect("plan should build");
    planner
        .approve(&proposal.task_id, &proposal.fingerprint, "fayeed")
        .expect("plan should approve");

    // The base moves after approval.
    fs::write(fixture.repository_root.join("new.txt"), "new\n").expect("file should write");
    git(&fixture.repository_root, &["add", "new.txt"]);
    git(
        &fixture.repository_root,
        &[
            "-c",
            "user.name=Mars Test",
            "-c",
            "user.email=mars@example.test",
            "commit",
            "--quiet",
            "-m",
            "moves the base",
        ],
    );

    let error = planner
        .provision_approved(&proposal.task_id)
        .expect_err("a plan whose base moved must not provision on the old approval");
    let message = error.to_string();
    assert!(
        message.contains("pointed at") && message.contains("now points at"),
        "the refusal should name both commits, got: {message}"
    );
    assert!(!proposal.worktree_path.exists());
}

#[test]
fn provisioning_an_approved_plan_creates_the_worktree_and_runs_its_setup() {
    let fixture = GitFixture::new();
    let (planner, activity) = planner(&fixture);
    let marker = if cfg!(windows) {
        "echo setup-ran> setup.txt"
    } else {
        "echo setup-ran > setup.txt"
    };
    let proposal = planner
        .propose(PlanRequest {
            setup_commands: vec![SetupCommand::new(marker, "prove setup runs")],
            ..request(&fixture, "Provisioned")
        })
        .expect("plan should build");
    planner
        .approve(&proposal.task_id, &proposal.fingerprint, "fayeed")
        .expect("plan should approve");

    let provisioned = planner
        .provision_approved(&proposal.task_id)
        .expect("approved plan should provision");

    assert_eq!(provisioned.state, WorktreeState::Ready);
    assert!(provisioned.worktree_path.join("README.md").is_file());
    assert!(
        provisioned.worktree_path.join("setup.txt").is_file(),
        "the approved setup command should have run in the new worktree"
    );
    assert!(
        git(&provisioned.worktree_path, &["branch", "--show-current"])
            .contains(&provisioned.branch_name)
    );

    let events = activity
        .read(&proposal.task_id, 0, 20)
        .expect("log should read");
    let kinds = events.iter().map(|event| event.kind).collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            ActivityKind::WorktreePlanned,
            ActivityKind::WorktreeCreated,
            ActivityKind::Command,
        ]
    );
    let setup = &events[2];
    assert_eq!(setup.payload["status"], "completed");
    assert_eq!(setup.payload["approvedWithPlan"], true);
    assert!(setup.duration_ms.is_some());
}

#[test]
fn provisioning_without_approval_is_refused() {
    let fixture = GitFixture::new();
    let (planner, _) = planner(&fixture);
    let proposal = planner
        .propose(request(&fixture, "Unapproved"))
        .expect("plan should build");

    let error = planner
        .provision_approved(&proposal.task_id)
        .expect_err("an unapproved plan must not provision");

    assert!(error.to_string().contains("has not been approved"));
    assert!(!proposal.worktree_path.exists());
}

#[test]
fn a_moved_base_is_reported_with_choices_that_state_their_consequences() {
    let fixture = GitFixture::new();
    let (planner, _) = planner(&fixture);
    let proposal = planner
        .propose(request(&fixture, "Moving base"))
        .expect("plan should build");

    assert!(
        planner
            .base_drift(&proposal.task_id)
            .expect("drift should read")
            .is_none(),
        "a base that has not moved is not drift"
    );

    fs::write(fixture.repository_root.join("moved.txt"), "moved\n").expect("file should write");
    git(&fixture.repository_root, &["add", "moved.txt"]);
    git(
        &fixture.repository_root,
        &[
            "-c",
            "user.name=Mars Test",
            "-c",
            "user.email=mars@example.test",
            "commit",
            "--quiet",
            "-m",
            "moves the base",
        ],
    );

    let drift = planner
        .base_drift(&proposal.task_id)
        .expect("drift should read")
        .expect("the base has moved");

    assert_eq!(drift.planned_oid, proposal.base_oid);
    assert_ne!(drift.current_oid, drift.planned_oid);
    assert_eq!(drift.commits_behind, 1);
    assert_eq!(
        drift.options.len(),
        2,
        "before provisioning, rebase is not one of the choices"
    );
    assert!(
        drift
            .options
            .iter()
            .all(|option| option.consequence.len() > 40)
    );
    assert!(drift.options.iter().any(|option| {
        option.choice == DriftChoice::ContinueAndDiverge
            && option
                .consequence
                .contains("will not contain the newer work")
    }));
}

#[test]
fn replanning_onto_a_moved_base_withdraws_the_earlier_approval() {
    let fixture = GitFixture::new();
    let (planner, _) = planner(&fixture);
    let proposal = planner
        .propose(request(&fixture, "Replan"))
        .expect("plan should build");
    planner
        .approve(&proposal.task_id, &proposal.fingerprint, "fayeed")
        .expect("plan should approve");

    fs::write(fixture.repository_root.join("moved.txt"), "moved\n").expect("file should write");
    git(&fixture.repository_root, &["add", "moved.txt"]);
    git(
        &fixture.repository_root,
        &[
            "-c",
            "user.name=Mars Test",
            "-c",
            "user.email=mars@example.test",
            "commit",
            "--quiet",
            "-m",
            "moves the base",
        ],
    );

    let replanned = planner
        .resolve_base_drift(&proposal.task_id, DriftChoice::Recreate, "fayeed")
        .expect("recreate should replan");

    assert_ne!(replanned.base_oid, proposal.base_oid);
    assert!(
        !replanned.is_approved(),
        "moving the plan to a new base withdraws the approval of the old one"
    );
    assert!(
        planner.provision_approved(&proposal.task_id).is_err(),
        "the replanned worktree needs approving again"
    );
}

#[test]
fn continuing_from_a_moved_base_keeps_the_plan_and_records_the_choice() {
    let fixture = GitFixture::new();
    let (planner, activity) = planner(&fixture);
    let proposal = planner
        .propose(request(&fixture, "Diverge"))
        .expect("plan should build");

    fs::write(fixture.repository_root.join("moved.txt"), "moved\n").expect("file should write");
    git(&fixture.repository_root, &["add", "moved.txt"]);
    git(
        &fixture.repository_root,
        &[
            "-c",
            "user.name=Mars Test",
            "-c",
            "user.email=mars@example.test",
            "commit",
            "--quiet",
            "-m",
            "moves the base",
        ],
    );

    let after = planner
        .resolve_base_drift(&proposal.task_id, DriftChoice::ContinueAndDiverge, "fayeed")
        .expect("diverging should be allowed");

    assert_eq!(
        after.base_oid, proposal.base_oid,
        "the approved base is kept"
    );
    let events = activity
        .read(&proposal.task_id, 0, 20)
        .expect("log should read");
    let choice = events.last().expect("the choice should be recorded");
    assert_eq!(choice.payload["driftChoice"], "continue_and_diverge");
    assert_eq!(choice.payload["decidedBy"], "fayeed");
}

#[test]
fn cleanup_says_what_it_will_remove_before_it_removes_anything() {
    let fixture = GitFixture::new();
    let (planner, _) = planner(&fixture);
    let proposal = planner
        .propose(request(&fixture, "Cleanup preview"))
        .expect("plan should build");
    planner
        .approve(&proposal.task_id, &proposal.fingerprint, "fayeed")
        .expect("plan should approve");
    let provisioned = planner
        .provision_approved(&proposal.task_id)
        .expect("plan should provision");
    fs::write(provisioned.worktree_path.join("scratch.txt"), "work\n")
        .expect("uncommitted work should write");

    let cleanup = planner
        .plan_cleanup(&proposal.task_id)
        .expect("cleanup should plan");

    assert_eq!(cleanup.branch_name, provisioned.branch_name);
    assert_eq!(cleanup.uncommitted, vec!["scratch.txt".to_owned()]);
    assert!(
        !cleanup.reversible,
        "uncommitted work cannot be brought back"
    );
    assert!(cleanup.archive_ref.starts_with("refs/mars/archive/"));
    assert!(
        provisioned.worktree_path.is_dir(),
        "planning removes nothing"
    );

    let error = planner
        .cleanup(&proposal.task_id, &cleanup.fingerprint, false, "fayeed")
        .expect_err("cleanup must not silently discard uncommitted work");
    assert!(error.to_string().contains("uncommitted"));
    assert!(provisioned.worktree_path.is_dir());
}

#[test]
fn a_cleaned_up_worktree_can_be_restored_from_its_archived_branch() {
    let fixture = GitFixture::new();
    let (planner, activity) = planner(&fixture);
    let proposal = planner
        .propose(request(&fixture, "Reversible cleanup"))
        .expect("plan should build");
    planner
        .approve(&proposal.task_id, &proposal.fingerprint, "fayeed")
        .expect("plan should approve");
    let provisioned = planner
        .provision_approved(&proposal.task_id)
        .expect("plan should provision");

    fs::write(
        provisioned.worktree_path.join("work.txt"),
        "committed work\n",
    )
    .expect("work should write");
    git(&provisioned.worktree_path, &["add", "work.txt"]);
    git(
        &provisioned.worktree_path,
        &[
            "-c",
            "user.name=Mars Test",
            "-c",
            "user.email=mars@example.test",
            "commit",
            "--quiet",
            "-m",
            "the agent's work",
        ],
    );
    let committed = git(&provisioned.worktree_path, &["rev-parse", "HEAD"])
        .trim()
        .to_owned();

    let cleanup = planner
        .plan_cleanup(&proposal.task_id)
        .expect("cleanup should plan");
    assert_eq!(cleanup.commits_ahead_of_base, 1);
    assert!(cleanup.reversible);

    let record = planner
        .cleanup(&proposal.task_id, &cleanup.fingerprint, true, "fayeed")
        .expect("clean worktree should remove");

    assert!(!provisioned.worktree_path.exists());
    assert_eq!(record.archived_oid.as_deref(), Some(committed.as_str()));
    assert!(
        git(
            &fixture.repository_root,
            &["rev-parse", &cleanup.archive_ref]
        )
        .contains(&committed),
        "the branch tip must survive removal"
    );

    let restored = planner
        .restore(&record.id, "fayeed")
        .expect("an archived worktree should restore");

    assert_eq!(restored.state, "restored");
    assert!(provisioned.worktree_path.join("work.txt").is_file());
    assert_eq!(
        git(&provisioned.worktree_path, &["rev-parse", "HEAD"]).trim(),
        committed
    );

    let events = activity
        .read(&proposal.task_id, 0, 30)
        .expect("log should read");
    assert!(
        events
            .iter()
            .any(|event| event.kind == ActivityKind::WorktreeRemoved)
    );
    assert!(
        events
            .iter()
            .any(|event| event.kind == ActivityKind::WorktreeRestored)
    );
}

#[test]
fn cleanup_refuses_a_fingerprint_that_no_longer_describes_the_worktree() {
    let fixture = GitFixture::new();
    let (planner, _) = planner(&fixture);
    let proposal = planner
        .propose(request(&fixture, "Stale cleanup"))
        .expect("plan should build");
    planner
        .approve(&proposal.task_id, &proposal.fingerprint, "fayeed")
        .expect("plan should approve");
    let provisioned = planner
        .provision_approved(&proposal.task_id)
        .expect("plan should provision");
    let cleanup = planner
        .plan_cleanup(&proposal.task_id)
        .expect("cleanup should plan");

    // The worktree changes after the preview was shown.
    fs::write(provisioned.worktree_path.join("late.txt"), "late\n").expect("file should write");

    let error = planner
        .cleanup(&proposal.task_id, &cleanup.fingerprint, true, "fayeed")
        .expect_err("a stale cleanup preview must be refused");
    assert!(error.to_string().contains("approved as plan"));
    assert!(provisioned.worktree_path.is_dir());
}

#[test]
fn amending_a_plan_reclassifies_it_and_withdraws_any_approval() {
    let fixture = GitFixture::new();
    let (planner, _) = planner(&fixture);
    let proposal = planner
        .propose(request(&fixture, "Edit before approving"))
        .expect("plan should build");
    let approved = planner
        .approve(&proposal.task_id, &proposal.fingerprint, "fayeed")
        .expect("plan should approve");
    assert!(approved.approved_at.is_some());

    let amended = planner
        .amend(
            &proposal.task_id,
            vec!["crates/**".into()],
            vec![SetupCommand::new("rm -rf /", "wipe the disk")],
        )
        .expect("plan should amend");

    // The edit is what the plan now declares.
    assert_eq!(amended.expected_scope, vec!["crates/**".to_owned()]);
    // The classification is the core's reading of the new command, not the
    // caller's claim about it.
    assert_eq!(amended.setup_commands[0].classification, "destructive");
    assert!(
        amended
            .preflight
            .iter()
            .any(|finding| finding.code == PreflightCode::RiskySetupCommand),
        "an amended plan is re-checked, not carried over"
    );

    // The plan that was said yes to no longer exists, so the yes is gone.
    assert!(amended.approved_at.is_none());
    assert!(amended.approved_by.is_none());
    assert_ne!(amended.fingerprint, proposal.fingerprint);
    assert!(
        planner.provision_approved(&proposal.task_id).is_err(),
        "an amended plan must be approved again before it can be provisioned"
    );
}

#[test]
fn a_provisioned_plan_cannot_be_amended() {
    let fixture = GitFixture::new();
    let (planner, _) = planner(&fixture);
    let proposal = planner
        .propose(request(&fixture, "Already on disk"))
        .expect("plan should build");
    planner
        .approve(&proposal.task_id, &proposal.fingerprint, "fayeed")
        .expect("plan should approve");
    planner
        .provision_approved(&proposal.task_id)
        .expect("plan should provision");

    assert!(
        planner
            .amend(&proposal.task_id, vec!["crates/**".into()], Vec::new())
            .is_err(),
        "the scope a worktree already exists under is not editable in place"
    );
}
