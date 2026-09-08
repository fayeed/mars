mod support;

use std::path::PathBuf;
use std::time::Duration;

use mars_activity::ActivityKind;
use mars_security::{
    BrokerDecision, BrokerOutcome, BrokerRequest, CapabilityCeiling, PermissionBroker, SideEffect,
};

use support::{BrokerFixture, runtime};

fn shell(command: &str, cwd: &PathBuf) -> SideEffect {
    SideEffect::Shell {
        command: command.to_owned(),
        cwd: cwd.clone(),
    }
}

fn implementer(task_id: &str, effect: SideEffect) -> BrokerRequest {
    BrokerRequest {
        task_id: task_id.to_owned(),
        run_id: None,
        agent_label: "claude".into(),
        ceiling: CapabilityCeiling::implementer(),
        effect,
    }
}

fn reviewer(task_id: &str, effect: SideEffect) -> BrokerRequest {
    BrokerRequest {
        task_id: task_id.to_owned(),
        run_id: None,
        agent_label: "codex (reviewer)".into(),
        ceiling: CapabilityCeiling::reviewer(),
        effect,
    }
}

fn allow(broker: &PermissionBroker, request: &BrokerRequest, decision: BrokerDecision) -> String {
    let (outcome, ticket) = broker.request(request).expect("request should be brokered");
    let BrokerOutcome::Pending { request_id } = outcome else {
        panic!("a first-time request should be pending, got {outcome:?}")
    };
    drop(ticket);
    match broker
        .decide(&request_id, decision, "fayeed")
        .expect("decision should record")
    {
        BrokerOutcome::Allowed { grant_id, .. } => grant_id,
        other => panic!("expected a grant, got {other:?}"),
    }
}

#[test]
fn a_reviewer_cannot_be_granted_a_write_by_anyone() {
    let fixture = BrokerFixture::new();
    let task = fixture.task("task-reviewer");
    let broker = fixture.broker();

    let (outcome, ticket) = broker
        .request(&reviewer(
            &task,
            SideEffect::FileWrite {
                path: fixture.worktree.join("src/main.ts"),
                declared_scope: vec!["src/**".into()],
            },
        ))
        .expect("request should be brokered");

    assert!(
        ticket.is_none(),
        "a refusal by construction must not leave anyone waiting on a human"
    );
    let BrokerOutcome::Denied {
        reason,
        by_construction,
    } = outcome
    else {
        panic!("a reviewer write must be denied")
    };
    assert!(by_construction);
    assert!(reason.contains("read-only"));
    assert!(
        broker
            .pending(&task)
            .expect("pending should read")
            .is_empty(),
        "a refused effect must never appear as a question for a human"
    );
    assert!(
        broker
            .grants(Some(&task))
            .expect("grants should read")
            .is_empty()
    );
}

#[test]
fn a_reviewer_may_still_ask_about_read_only_commands() {
    let fixture = BrokerFixture::new();
    let task = fixture.task("task-reviewer-reads");
    let broker = fixture.broker();

    let (read_outcome, _ticket) = broker
        .request(&reviewer(&task, shell("git diff HEAD", &fixture.worktree)))
        .expect("request should be brokered");
    let (write_outcome, _) = broker
        .request(&reviewer(&task, shell("npm install", &fixture.worktree)))
        .expect("request should be brokered");

    assert!(matches!(read_outcome, BrokerOutcome::Pending { .. }));
    let BrokerOutcome::Denied {
        by_construction,
        reason,
    } = write_outcome
    else {
        panic!("a reviewer must not run a network command")
    };
    assert!(by_construction);
    assert!(reason.contains("network"));
}

#[test]
fn a_blocked_agent_waits_until_a_person_decides() {
    let fixture = BrokerFixture::new();
    let task = fixture.task("task-blocked");
    let broker = fixture.broker();

    runtime().block_on(async {
        let (outcome, ticket) = broker
            .request(&implementer(&task, shell("npm test", &fixture.worktree)))
            .expect("request should be brokered");
        let BrokerOutcome::Pending { request_id } = outcome else {
            panic!("a first-time request should be pending")
        };
        let ticket = ticket.expect("a pending request should hand back a ticket");
        assert_eq!(ticket.request_id(), request_id);

        let waiting = tokio::spawn(async move { ticket.resolve().await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(
            !waiting.is_finished(),
            "an agent must stay blocked while nobody has answered"
        );

        let pending = broker.pending(&task).expect("pending should read");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, request_id);
        assert_eq!(
            pending[0].summary,
            format!("run `npm test` in {}", fixture.worktree.display())
        );

        broker
            .decide(&request_id, BrokerDecision::AllowOnce, "fayeed")
            .expect("decision should record");

        let resolved = tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("the ticket should resolve once a person answers")
            .expect("the waiting task should not panic")
            .expect("the ticket should carry the outcome");
        assert!(matches!(resolved, BrokerOutcome::Allowed { .. }));
        assert!(
            broker
                .pending(&task)
                .expect("pending should read")
                .is_empty()
        );
    });
}

#[test]
fn allow_once_covers_exactly_one_later_request() {
    let fixture = BrokerFixture::new();
    let task = fixture.task("task-once");
    let broker = fixture.broker();
    let request = implementer(&task, shell("npm test", &fixture.worktree));

    allow(&broker, &request, BrokerDecision::AllowOnce);

    let (second, _) = broker.request(&request).expect("second should broker");
    assert!(
        matches!(second, BrokerOutcome::Allowed { .. }),
        "the single use should cover the next request"
    );
    let (third, _) = broker.request(&request).expect("third should broker");
    assert!(
        matches!(third, BrokerOutcome::Pending { .. }),
        "a spent allow-once grant must ask again, got {third:?}"
    );
}

#[test]
fn allow_for_this_task_does_not_leak_into_another_task() {
    let fixture = BrokerFixture::new();
    let task = fixture.task("task-scoped");
    let other = fixture.task("task-other");
    let broker = fixture.broker();

    allow(
        &broker,
        &implementer(&task, shell("npm test", &fixture.worktree)),
        BrokerDecision::AllowTask,
    );

    let (same_task, _) = broker
        .request(&implementer(&task, shell("npm test", &fixture.worktree)))
        .expect("same task should broker");
    let (other_task, _) = broker
        .request(&implementer(&other, shell("npm test", &fixture.worktree)))
        .expect("other task should broker");

    assert!(matches!(same_task, BrokerOutcome::Allowed { .. }));
    assert!(
        matches!(other_task, BrokerOutcome::Pending { .. }),
        "a task grant must not answer for a different task"
    );
}

#[test]
fn allow_for_this_pattern_covers_matching_commands_and_nothing_else() {
    let fixture = BrokerFixture::new();
    let task = fixture.task("task-pattern");
    let broker = fixture.broker();

    allow(
        &broker,
        &implementer(&task, shell("npm test src/math.test.ts", &fixture.worktree)),
        BrokerDecision::AllowPattern,
    );

    let (matching, _) = broker
        .request(&implementer(
            &task,
            shell("npm test src/other.test.ts", &fixture.worktree),
        ))
        .expect("matching command should broker");
    let (different, _) = broker
        .request(&implementer(
            &task,
            shell("npm run build", &fixture.worktree),
        ))
        .expect("different command should broker");

    assert!(
        matches!(matching, BrokerOutcome::Allowed { .. }),
        "`npm test *` should cover another `npm test`, got {matching:?}"
    );
    assert!(
        matches!(different, BrokerOutcome::Pending { .. }),
        "`npm test *` must not cover `npm run build`, got {different:?}"
    );
}

#[test]
fn a_pattern_grant_reaches_across_tasks_but_a_task_grant_does_not() {
    let fixture = BrokerFixture::new();
    let task = fixture.task("task-pattern-wide");
    let other = fixture.task("task-pattern-other");
    let broker = fixture.broker();

    allow(
        &broker,
        &implementer(&task, shell("git status", &fixture.worktree)),
        BrokerDecision::AllowPattern,
    );

    let (elsewhere, _) = broker
        .request(&implementer(
            &other,
            shell("git status -sb", &fixture.worktree),
        ))
        .expect("other task should broker");

    assert!(matches!(elsewhere, BrokerOutcome::Allowed { .. }));
}

#[test]
fn denying_grants_nothing_and_asks_again_next_time() {
    let fixture = BrokerFixture::new();
    let task = fixture.task("task-denied");
    let broker = fixture.broker();
    let request = implementer(&task, shell("rm -rf build", &fixture.worktree));

    let (outcome, _) = broker.request(&request).expect("request should broker");
    let BrokerOutcome::Pending { request_id } = outcome else {
        panic!("a destructive command should be asked about")
    };
    let decided = broker
        .decide(&request_id, BrokerDecision::Deny, "fayeed")
        .expect("denial should record");

    let BrokerOutcome::Denied {
        by_construction,
        reason,
    } = decided
    else {
        panic!("a denial should be denied")
    };
    assert!(!by_construction, "a person's no is not a ceiling refusal");
    assert!(reason.contains("fayeed"));
    assert!(
        broker
            .grants(Some(&task))
            .expect("grants should read")
            .is_empty()
    );

    let (again, _) = broker.request(&request).expect("request should broker");
    assert!(matches!(again, BrokerOutcome::Pending { .. }));
}

#[test]
fn a_decision_can_only_be_made_once() {
    let fixture = BrokerFixture::new();
    let task = fixture.task("task-twice");
    let broker = fixture.broker();

    let (outcome, _) = broker
        .request(&implementer(&task, shell("npm test", &fixture.worktree)))
        .expect("request should broker");
    let BrokerOutcome::Pending { request_id } = outcome else {
        panic!("expected a pending request")
    };
    broker
        .decide(&request_id, BrokerDecision::AllowOnce, "fayeed")
        .expect("first decision should record");

    assert!(
        broker
            .decide(&request_id, BrokerDecision::Deny, "someone-else")
            .is_err(),
        "a decided request must not be re-decided"
    );
    assert!(
        broker
            .decide("no-such-request", BrokerDecision::Deny, "fayeed")
            .is_err()
    );
}

#[test]
fn revoking_a_grant_stops_it_covering_new_work_and_keeps_its_history() {
    let fixture = BrokerFixture::new();
    let task = fixture.task("task-revoke");
    let broker = fixture.broker();
    let request = implementer(&task, shell("npm test", &fixture.worktree));
    let grant_id = allow(&broker, &request, BrokerDecision::AllowTask);

    broker
        .revoke(&grant_id, "fayeed")
        .expect("grant should revoke");

    let (after, _) = broker.request(&request).expect("request should broker");
    assert!(
        matches!(after, BrokerOutcome::Pending { .. }),
        "a revoked grant must stop covering requests"
    );

    let grants = broker.grants(Some(&task)).expect("grants should read");
    let revoked = grants
        .iter()
        .find(|grant| grant.id == grant_id)
        .expect("a revoked grant stays listed");
    assert!(!revoked.active);
    assert_eq!(revoked.revoked_by.as_deref(), Some("fayeed"));
    assert_eq!(revoked.granted_by, "fayeed");
    assert!(
        revoked.expires_at.is_some(),
        "a grant should carry an expiry"
    );
    assert!(
        broker.revoke(&grant_id, "fayeed").is_err(),
        "revoking twice should report that there was nothing to revoke"
    );
}

#[test]
fn an_expired_grant_no_longer_answers_for_the_agent() {
    let fixture = BrokerFixture::new();
    let task = fixture.task("task-expiry");
    let broker = fixture.broker();
    let request = implementer(&task, shell("npm test", &fixture.worktree));
    let grant_id = allow(&broker, &request, BrokerDecision::AllowTask);

    let (before, _) = broker.request(&request).expect("request should broker");
    assert!(matches!(before, BrokerOutcome::Allowed { .. }));

    fixture.expire_grant(&grant_id);

    let (after, _) = broker.request(&request).expect("request should broker");
    assert!(
        matches!(after, BrokerOutcome::Pending { .. }),
        "an expired grant must ask again"
    );
    assert!(
        !broker
            .grants(Some(&task))
            .expect("grants should read")
            .iter()
            .any(|grant| grant.active)
    );
}

#[test]
fn the_transcript_records_who_decided_what_and_when() {
    let fixture = BrokerFixture::new();
    let task = fixture.task("task-transcript");
    let broker = fixture.broker();
    let record = fixture.activity();

    let (outcome, _) = broker
        .request(&implementer(
            &task,
            shell("rm -rf build", &fixture.worktree),
        ))
        .expect("request should broker");
    let BrokerOutcome::Pending { request_id } = outcome else {
        panic!("expected a pending request")
    };
    broker
        .decide(&request_id, BrokerDecision::AllowOnce, "fayeed")
        .expect("decision should record");

    let events = record.read(&task, 0, 100).expect("log should read");
    assert_eq!(events.len(), 2);

    let asked = &events[0];
    assert_eq!(asked.kind, ActivityKind::PermissionRequest);
    assert_eq!(asked.actor_label, "claude");
    assert_eq!(asked.payload["requestId"], serde_json::json!(request_id));
    assert_eq!(
        asked.payload["classification"],
        serde_json::json!("destructive")
    );
    assert!(
        asked.payload["reason"]
            .as_str()
            .expect("a request should say why it is being asked")
            .contains("delete a directory tree")
    );

    let decided = &events[1];
    assert_eq!(decided.kind, ActivityKind::PermissionDecision);
    assert_eq!(decided.actor, mars_activity::Actor::Human);
    assert_eq!(decided.actor_label, "fayeed");
    assert_eq!(decided.payload["decision"], serde_json::json!("allow_once"));
    assert_eq!(decided.payload["byConstruction"], serde_json::json!(false));
    assert!(!decided.occurred_at.is_empty());
}

#[test]
fn a_refusal_by_construction_is_recorded_as_the_systems_decision() {
    let fixture = BrokerFixture::new();
    let task = fixture.task("task-ceiling-log");
    let broker = fixture.broker();
    let record = fixture.activity();

    broker
        .request(&reviewer(
            &task,
            SideEffect::GitPush {
                remote: "origin".into(),
                branch: "mars/work".into(),
                cwd: fixture.worktree.clone(),
            },
        ))
        .expect("request should broker");

    let events = record.read(&task, 0, 100).expect("log should read");
    let decision = events
        .iter()
        .find(|event| event.kind == ActivityKind::PermissionDecision)
        .expect("a ceiling refusal should still be recorded");

    assert_eq!(decision.actor, mars_activity::Actor::System);
    assert_eq!(decision.payload["byConstruction"], serde_json::json!(true));
    assert!(decision.summary.starts_with("Refused:"));
    assert!(
        !events
            .iter()
            .any(|event| event.kind == ActivityKind::PermissionRequest),
        "a refused effect is never posed as a question"
    );
}

#[test]
fn a_write_inside_a_granted_directory_is_covered_but_one_outside_is_not() {
    let fixture = BrokerFixture::new();
    let task = fixture.task("task-paths");
    let broker = fixture.broker();

    allow(
        &broker,
        &implementer(
            &task,
            SideEffect::FileWrite {
                path: fixture.worktree.clone(),
                declared_scope: vec!["src/**".into()],
            },
        ),
        BrokerDecision::AllowTask,
    );

    let (inside, _) = broker
        .request(&implementer(
            &task,
            SideEffect::FileWrite {
                path: fixture.worktree.join("src").join("main.ts"),
                declared_scope: vec!["src/**".into()],
            },
        ))
        .expect("inside should broker");
    let (outside, _) = broker
        .request(&implementer(
            &task,
            SideEffect::FileWrite {
                path: PathBuf::from("/etc/hosts"),
                declared_scope: vec!["src/**".into()],
            },
        ))
        .expect("outside should broker");

    assert!(matches!(inside, BrokerOutcome::Allowed { .. }));
    assert!(
        matches!(outside, BrokerOutcome::Pending { .. }),
        "a grant over the worktree must not cover a path outside it"
    );
}
