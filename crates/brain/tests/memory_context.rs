//! Where memory meets the agent: the context pack.
//!
//! Lifecycle point three says memories are injected into agent context and
//! every injection is logged. These tests are the proof that the injection is
//! real rather than an API nobody calls, and that the thing being injected is
//! always something a human approved.

mod support;

use mars_brain::{
    ContextPack, ContextPackRequest, ContextPackService, InjectionContext, MemoryKind, MemoryScope,
    NewProposal, PackSection, injections_from_pack,
};

use support::MemoryFixture;

fn pack_request(fixture: &MemoryFixture) -> ContextPackRequest {
    ContextPackRequest {
        task_id: fixture.task_id.clone(),
        adapter: "claude".into(),
        max_bytes: 16 * 1024,
        excluded_entity_refs: Vec::new(),
    }
}

/// What the agent launch path does once the rebuilt pack has been checked
/// against the preview: record exactly the memories the sent pack carried.
fn launch(fixture: &MemoryFixture, pack: &ContextPack) {
    fixture
        .service()
        .record_uses(
            &injections_from_pack(pack),
            &InjectionContext {
                task_id: Some(fixture.task_id.clone()),
                session_id: None,
                surface: "context_pack".into(),
            },
        )
        .expect("recording the injection should succeed");
}

#[test]
fn an_approved_memory_reaches_the_agent_as_an_instruction() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    let memory = fixture.approve(&service, "run the database migrations before the tests");

    let pack = ContextPackService::new(fixture.storage.clone())
        .build(pack_request(&fixture))
        .expect("pack should build");

    let item = pack
        .items
        .iter()
        .find(|item| item.entity_ref == format!("memory:{}", memory.id))
        .unwrap_or_else(|| {
            panic!(
                "the approved memory should be in the pack: {:#?}",
                pack.items
                    .iter()
                    .map(|item| &item.entity_ref)
                    .collect::<Vec<_>>()
            )
        });

    // A convention is an instruction, not background reading.
    assert_eq!(item.section, PackSection::Instructions);
    assert_eq!(item.content, "run the database migrations before the tests");
    // The agent sees that a person stands behind this, and where it came from.
    assert!(
        item.provenance.contains("HUMAN") && item.provenance.contains("fayeed"),
        "provenance should name the approver: {}",
        item.provenance
    );
    assert!(
        item.source_location.starts_with("conversation:"),
        "an injected memory stays traceable to its turn: {}",
        item.source_location
    );

    // Launching records exactly what that pack carried.
    launch(&fixture, &pack);
    let usage = service.usage(&memory.id).expect("usage should load");
    assert_eq!(usage.use_count, 1);
    assert_eq!(usage.uses[0].surface, "context_pack");
    assert_eq!(
        usage.uses[0].task_id.as_deref(),
        Some(fixture.task_id.as_str())
    );
}

#[test]
fn a_pending_proposal_never_reaches_the_agent() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    service
        .propose(NewProposal {
            project_id: fixture.project_id.clone(),
            repository_id: Some(fixture.repository_id.clone()),
            task_id: None,
            scope: MemoryScope::Repo,
            kind: MemoryKind::Convention,
            text: "run the database migrations before the tests".into(),
            confidence: 0.95,
            conversation_id: "conversation-1".into(),
            message_id: "message-1".into(),
            quoted_text: "the human said: run the migrations first".into(),
            proposed_by: "claude".into(),
        })
        .expect("proposal should queue");

    let pack = ContextPackService::new(fixture.storage.clone())
        .build(pack_request(&fixture))
        .expect("pack should build");

    assert!(
        !pack
            .items
            .iter()
            .any(|item| item.content.contains("run the database migrations")),
        "an unapproved proposal must never reach an agent: {:#?}",
        pack.items
            .iter()
            .map(|item| &item.content)
            .collect::<Vec<_>>()
    );
}

#[test]
fn building_a_pack_is_not_using_a_memory_but_launching_is() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    let memory = fixture.approve(&service, "run the database migrations before the tests");

    let preview = ContextPackService::new(fixture.storage.clone())
        .build(pack_request(&fixture))
        .expect("preview should build");
    assert!(
        preview
            .items
            .iter()
            .any(|item| item.entity_ref == format!("memory:{}", memory.id)),
        "the preview should show what would be injected"
    );
    assert_eq!(
        service.usage(&memory.id).expect("usage").use_count,
        0,
        "a pack the human is still editing has shaped nothing"
    );

    // ADR-021 rebuilds the pack and compares it before launch. That rebuild is
    // still not an injection, and must be byte-identical to the preview.
    let rebuilt = ContextPackService::new(fixture.storage.clone())
        .build(pack_request(&fixture))
        .expect("rebuild should succeed");
    assert_eq!(rebuilt, preview, "revalidation must not change the pack");
    assert_eq!(
        service.usage(&memory.id).expect("usage").use_count,
        0,
        "revalidating is not injecting"
    );

    launch(&fixture, &rebuilt);
    assert_eq!(
        service.usage(&memory.id).expect("usage").use_count,
        1,
        "exactly one injection is recorded for one launch"
    );
}

#[test]
fn a_launch_records_only_what_the_sent_pack_carried() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    let kept = fixture.approve(&service, "run the database migrations before the tests");
    let removed = fixture.approve(&service, "the release notes live in a separate repository");

    // The human removed one memory from the preview before launching.
    let pack = ContextPackService::new(fixture.storage.clone())
        .build(ContextPackRequest {
            excluded_entity_refs: vec![format!("memory:{}", removed.id)],
            ..pack_request(&fixture)
        })
        .expect("pack should build");
    launch(&fixture, &pack);

    assert_eq!(service.usage(&kept.id).expect("usage").use_count, 1);
    assert_eq!(
        service.usage(&removed.id).expect("usage").use_count,
        0,
        "a memory the human took out of the pack was never injected"
    );
}

#[test]
fn a_revoked_memory_disappears_from_agent_context() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    let memory = fixture.approve(&service, "run the database migrations before the tests");
    service
        .revoke(&mars_brain::RevokeRequest {
            memory_id: memory.id.clone(),
            revoked_by: "fayeed".into(),
            reason: "migrations now run in CI".into(),
        })
        .expect("revoke should succeed");

    let pack = ContextPackService::new(fixture.storage.clone())
        .build(pack_request(&fixture))
        .expect("pack should build");
    assert!(
        !pack
            .items
            .iter()
            .any(|item| item.entity_ref == format!("memory:{}", memory.id)),
        "withdrawing a memory must stop it reaching agents"
    );
}

#[test]
fn a_caution_that_reaches_packing_survives_a_tight_section_budget() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    // Few enough memories that retrieval returns all of them, so the only
    // thing under test is what the byte budget does. Retrieval itself ranks
    // every kind equally, and a caution beyond the retrieval limit is cut like
    // any other memory -- see the limits section of the Phase 2 proof.
    let conventions: Vec<_> = (0..5)
        .map(|index| {
            fixture.approve_kind(
                &service,
                &format!(
                    "convention {index}: keep the migration runner and the test harness \
                     aligned, and prefer the shared fixture helpers over bespoke setup"
                ),
                MemoryKind::Convention,
                &format!("conversation-{index}"),
            )
        })
        .collect();
    let caution = fixture.approve_kind(
        &service,
        "the payment provider rate-limits at 100 requests a minute during migrations",
        MemoryKind::Caution,
        "conversation-caution",
    );

    let pack = ContextPackService::new(fixture.storage.clone())
        .build(ContextPackRequest {
            max_bytes: 2_048,
            ..pack_request(&fixture)
        })
        .expect("pack should build");

    let packed: Vec<&String> = pack.items.iter().map(|item| &item.entity_ref).collect();
    assert!(
        packed.contains(&&format!("memory:{}", caution.id)),
        "the memory whose absence could cause harm is the one that survives: {packed:#?}"
    );
    assert!(
        conventions
            .iter()
            .any(|convention| !packed.contains(&&format!("memory:{}", convention.id))),
        "the budget should actually have been tight enough to drop something"
    );
}
