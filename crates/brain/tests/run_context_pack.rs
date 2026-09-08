//! The context pack as the thing a person inspects before a run starts.

mod support;

use mars_brain::{RunContextPackBuilder, RunPackRequest, ToolGrant};
use mars_security::{
    BrokerDecision, BrokerOutcome, BrokerRequest, CapabilityCeiling, PermissionBroker, SideEffect,
};

use support::indexed_fixture;

fn tools() -> Vec<ToolGrant> {
    vec![
        ToolGrant::read_only("read_file", "read a file in the worktree"),
        ToolGrant::read_only("search", "search the index"),
        ToolGrant::mutating("write_file", "write a file in the worktree"),
    ]
}

fn request(task_id: &str, role: &str, ceiling: CapabilityCeiling) -> RunPackRequest {
    RunPackRequest {
        task_id: task_id.to_owned(),
        agent_id: format!("claude:{role}"),
        adapter: "claude".into(),
        role: role.to_owned(),
        ceiling,
        declared_scope: vec!["src/**".into()],
        tools: tools(),
        assignment: "Add a guard to add().".into(),
        max_bytes: 24 * 1024,
        excluded_entity_refs: Vec::new(),
    }
}

fn task(fixture: &support::BrainFixture, id: &str) -> String {
    fixture
        .storage
        .with_transaction(|transaction| {
            let project_id: String = transaction.query_row(
                "SELECT project_id FROM repositories WHERE id = ?1",
                [&fixture.repository_id],
                |row| row.get(0),
            )?;
            transaction.execute(
                "INSERT INTO tasks
                     (id, project_id, repository_id, title, intent, status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 'Guard addition', 'Make add reject unsafe sums', 'ready',
                         '2026-09-08T00:00:00Z', '2026-09-08T00:00:00Z')",
                rusqlite::params![id, project_id, &fixture.repository_id],
            )?;
            Ok(())
        })
        .expect("task should insert");
    id.to_owned()
}

#[test]
fn the_pack_states_the_tools_and_permissions_the_agent_will_have() {
    let fixture = indexed_fixture();
    let task_id = task(&fixture, "task-pack");
    let builder = RunContextPackBuilder::new(fixture.storage.clone());

    let pack = builder
        .build(request(
            &task_id,
            "implementer",
            CapabilityCeiling::implementer(),
        ))
        .expect("pack should build");

    assert_eq!(pack.tools.len(), 3);
    assert_eq!(pack.permissions.role, "implementer");
    assert_eq!(pack.permissions.declared_scope, vec!["src/**"]);
    assert!(pack.permissions.everything_else_asks);
    assert!(pack.permissions.ceiling.write_files);

    // The prompt is the artefact, not a summary of one: what the reader sees is
    // what the agent is sent.
    assert!(pack.prompt.contains("## Tools"));
    assert!(pack.prompt.contains("write_file (may change files)"));
    assert!(pack.prompt.contains("Declared file scope: src/**."));
    assert!(pack.prompt.contains("You may write files"));
    assert!(pack.prompt.contains("Add a guard to add()."));
    assert!(
        pack.prompt
            .contains("Anything else stops you and asks a person")
    );
    assert_eq!(pack.total_bytes, pack.prompt.len());
    assert_eq!(pack.digest.len(), 64);
}

#[test]
fn a_reviewers_pack_never_offers_a_tool_that_would_be_refused() {
    let fixture = indexed_fixture();
    let task_id = task(&fixture, "task-reviewer-pack");
    let builder = RunContextPackBuilder::new(fixture.storage.clone());

    let pack = builder
        .build(request(&task_id, "reviewer", CapabilityCeiling::reviewer()))
        .expect("pack should build");

    let names = pack
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["read_file", "search"]);
    assert!(!pack.prompt.contains("write_file"));
    assert!(pack.prompt.contains("You may not write files"));
    assert!(pack.prompt.contains("not reach the network"));
    assert!(pack.prompt.contains("not push"));
}

#[test]
fn standing_grants_are_shown_so_a_reader_knows_what_will_not_be_asked() {
    let fixture = indexed_fixture();
    let task_id = task(&fixture, "task-grants");
    let broker = PermissionBroker::new(fixture.storage.clone());

    let (outcome, _ticket) = broker
        .request(&BrokerRequest {
            task_id: task_id.clone(),
            run_id: None,
            agent_label: "claude".into(),
            ceiling: CapabilityCeiling::implementer(),
            effect: SideEffect::Shell {
                command: "npm test".into(),
                cwd: fixture.root.clone(),
            },
        })
        .expect("request should broker");
    let BrokerOutcome::Pending { request_id } = outcome else {
        panic!("expected a pending request")
    };
    broker
        .decide(&request_id, BrokerDecision::AllowTask, "fayeed")
        .expect("decision should record");

    let pack = RunContextPackBuilder::new(fixture.storage.clone())
        .with_grants(broker.grants(Some(&task_id)).expect("grants should read"))
        .build(request(
            &task_id,
            "implementer",
            CapabilityCeiling::implementer(),
        ))
        .expect("pack should build");

    assert_eq!(pack.permissions.standing_grants.len(), 1);
    assert_eq!(pack.permissions.standing_grants[0].granted_by, "fayeed");
    assert_eq!(pack.permissions.standing_grants[0].decision, "allow_task");
    assert!(pack.prompt.contains("already allowed: run_command"));
    assert!(pack.prompt.contains("granted by fayeed"));
}

#[test]
fn the_same_inputs_produce_the_same_pack_and_the_digest_says_so() {
    let fixture = indexed_fixture();
    let task_id = task(&fixture, "task-stable");
    let builder = RunContextPackBuilder::new(fixture.storage.clone());

    let first = builder
        .build(request(
            &task_id,
            "implementer",
            CapabilityCeiling::implementer(),
        ))
        .expect("pack should build");
    let second = builder
        .build(request(
            &task_id,
            "implementer",
            CapabilityCeiling::implementer(),
        ))
        .expect("pack should rebuild");

    assert_eq!(first.digest, second.digest);
    assert_eq!(first.prompt, second.prompt);

    builder
        .revalidate(
            request(&task_id, "implementer", CapabilityCeiling::implementer()),
            &first.digest,
        )
        .expect("an unchanged pack should revalidate");
}

#[test]
fn a_pack_that_changed_after_approval_is_refused_at_launch() {
    let fixture = indexed_fixture();
    let task_id = task(&fixture, "task-revalidate");
    let builder = RunContextPackBuilder::new(fixture.storage.clone());
    let approved = builder
        .build(request(
            &task_id,
            "implementer",
            CapabilityCeiling::implementer(),
        ))
        .expect("pack should build");

    // The role is narrowed after approval, which changes what will be sent.
    let error = builder
        .revalidate(
            request(&task_id, "reviewer", CapabilityCeiling::reviewer()),
            &approved.digest,
        )
        .expect_err("a changed pack must not be sent on an old approval");

    assert!(
        error.to_string().contains("approved as"),
        "the refusal should name both digests, got: {error}"
    );
}

#[test]
fn every_included_item_says_where_it_came_from_and_why() {
    let fixture = indexed_fixture();
    let task_id = task(&fixture, "task-provenance");

    let pack = RunContextPackBuilder::new(fixture.storage.clone())
        .build(request(
            &task_id,
            "implementer",
            CapabilityCeiling::implementer(),
        ))
        .expect("pack should build");

    for entry in &pack.entries {
        assert!(!entry.entity_ref.is_empty());
        assert!(!entry.provenance.is_empty(), "{entry:?} has no provenance");
        assert!(!entry.reason.is_empty(), "{entry:?} has no reason");
        assert!(!entry.kind.is_empty());
        assert!(entry.byte_size > 0);
    }
    // The index groups by what a thing is, so a reader can ask "which files?".
    let grouped = pack.of_kind("task");
    assert!(
        grouped.len() <= pack.entries.len(),
        "grouping must not invent entries"
    );
}
