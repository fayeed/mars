//! Memory: human approval, traceability, and the lifecycle around them.
//!
//! The two rules under test are that nothing becomes a memory without a
//! person approving it, and that every memory can be traced to the turn it
//! came from. Most of these tests exist to try to violate one of those.

mod support;

use std::sync::Arc;

use mars_brain::{
    ApprovalOutcome, ApproveRequest, ConflictChoice, ConflictResolution, ConflictSignal,
    MemoryError, MemoryEventKind, MemoryFilters, MemoryKind, MemoryScope, MemoryService,
    MemoryStatus, NewProposal, ProposalFilters, ProposalStatus, RejectRequest, RetrievalContext,
    ReviseRequest, RevokeRequest,
};

use support::MemoryFixture;

fn proposal(fixture: &MemoryFixture, text: &str) -> NewProposal {
    NewProposal {
        project_id: fixture.project_id.clone(),
        repository_id: Some(fixture.repository_id.clone()),
        task_id: None,
        scope: MemoryScope::Repo,
        kind: MemoryKind::Convention,
        text: text.to_owned(),
        confidence: 0.8,
        conversation_id: "conversation-1".into(),
        message_id: "message-1".into(),
        quoted_text: format!("the human said: {text}"),
        proposed_by: "claude".into(),
    }
}

// --- proposal ------------------------------------------------------------

#[test]
fn a_proposal_is_queued_and_never_becomes_a_memory_on_its_own() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();

    let queued = service
        .propose(proposal(&fixture, "run the formatter before committing"))
        .expect("proposal should queue");
    assert_eq!(queued.status, ProposalStatus::Pending);
    assert!(queued.memory_id.is_none());

    // Nothing an agent can call has produced a memory.
    let memories = service
        .list(&MemoryFilters {
            project_id: fixture.project_id.clone(),
            ..MemoryFilters::default()
        })
        .expect("list should work");
    assert!(
        memories.is_empty(),
        "a proposal must not be retrievable as a memory"
    );

    let pending = service
        .pending(&ProposalFilters {
            project_id: fixture.project_id.clone(),
            ..ProposalFilters::default()
        })
        .expect("pending should work");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, queued.id);
}

#[test]
fn a_proposal_without_its_motivating_excerpt_is_refused() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    let mut input = proposal(&fixture, "prefer explicit imports");
    input.quoted_text = "   ".into();

    let error = service
        .propose(input)
        .expect_err("a memory nobody can trace is not a memory");
    assert!(
        matches!(error, MemoryError::Empty { field: "quotedText" }),
        "got {error}"
    );
}

#[test]
fn the_same_claim_from_the_same_turn_is_one_proposal() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    service
        .propose(proposal(&fixture, "prefer explicit imports"))
        .expect("first proposal should queue");
    let error = service
        .propose(proposal(&fixture, "prefer explicit imports"))
        .expect_err("a restatement of the same turn is not a second proposal");
    assert!(matches!(error, MemoryError::Storage(_)), "got {error}");
}

#[test]
fn a_scope_that_cannot_be_resolved_is_refused() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    let mut input = proposal(&fixture, "this task uses the legacy client");
    input.scope = MemoryScope::Task;
    input.task_id = None;

    let error = service.propose(input).expect_err("task scope needs a task");
    assert!(
        matches!(error, MemoryError::ScopeIncomplete { scope: "task", .. }),
        "got {error}"
    );
}

// --- approval ------------------------------------------------------------

#[test]
fn approval_records_who_approved_and_makes_the_memory_retrievable() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    let queued = service
        .propose(proposal(&fixture, "run the formatter before committing"))
        .expect("proposal should queue");

    let outcome = service
        .approve(&ApproveRequest {
            proposal_id: queued.id.clone(),
            approved_by: "fayeed".into(),
            edited_text: None,
            resolutions: Vec::new(),
        })
        .expect("approval should succeed");

    let ApprovalOutcome::Approved { memory, message } = outcome else {
        panic!("expected an approval, got {outcome:?}")
    };
    assert_eq!(memory.approved_by, "fayeed");
    assert!(!memory.approved_at.is_empty());
    assert_eq!(memory.status, MemoryStatus::Active);
    assert_eq!(memory.text, "run the formatter before committing");
    // Traceable: the origin survives approval intact.
    assert_eq!(memory.origin.conversation_id, "conversation-1");
    assert_eq!(memory.origin.message_id, "message-1");
    assert!(memory.origin.quoted_text.contains("run the formatter"));

    // The approval is itself a message in the origin conversation.
    assert_eq!(message.kind, MemoryEventKind::Approved);
    assert_eq!(message.conversation_id, "conversation-1");
    assert_eq!(message.actor, "fayeed");
    assert_ne!(
        message.message_id, "message-1",
        "the approval is a new turn, not the one being quoted"
    );

    let reloaded = service.proposal(&queued.id).expect("proposal should reload");
    assert_eq!(reloaded.status, ProposalStatus::Approved);
    assert_eq!(reloaded.memory_id.as_deref(), Some(memory.id.as_str()));
}

#[test]
fn editing_before_accepting_stores_the_edit_and_says_so() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    let queued = service
        .propose(proposal(&fixture, "always use tabs"))
        .expect("proposal should queue");

    let outcome = service
        .approve(&ApproveRequest {
            proposal_id: queued.id.clone(),
            approved_by: "fayeed".into(),
            edited_text: Some("always use spaces, never tabs".into()),
            resolutions: Vec::new(),
        })
        .expect("edited approval should succeed");

    let ApprovalOutcome::Approved { memory, message } = outcome else {
        panic!("expected an approval")
    };
    assert_eq!(memory.text, "always use spaces, never tabs");
    assert_eq!(message.kind, MemoryEventKind::EditedAndApproved);
    // The excerpt still points at what the agent originally heard.
    assert!(memory.origin.quoted_text.contains("always use tabs"));
}

#[test]
fn approval_requires_a_named_approver() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    let queued = service
        .propose(proposal(&fixture, "prefer explicit imports"))
        .expect("proposal should queue");

    let error = service
        .approve(&ApproveRequest {
            proposal_id: queued.id,
            approved_by: "  ".into(),
            edited_text: None,
            resolutions: Vec::new(),
        })
        .expect_err("an unattributed approval is not an approval");
    assert!(
        matches!(error, MemoryError::Empty { field: "approvedBy" }),
        "got {error}"
    );
}

#[test]
fn a_proposal_cannot_be_approved_twice() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    let queued = service
        .propose(proposal(&fixture, "prefer explicit imports"))
        .expect("proposal should queue");
    let request = ApproveRequest {
        proposal_id: queued.id.clone(),
        approved_by: "fayeed".into(),
        edited_text: None,
        resolutions: Vec::new(),
    };
    service.approve(&request).expect("first approval succeeds");
    let error = service
        .approve(&request)
        .expect_err("a resolved proposal is closed");
    assert!(
        matches!(error, MemoryError::ProposalNotPending { .. }),
        "got {error}"
    );
}

#[test]
fn rejection_closes_the_proposal_and_writes_no_memory() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    let queued = service
        .propose(proposal(&fixture, "delete the tests"))
        .expect("proposal should queue");

    let rejected = service
        .reject(&RejectRequest {
            proposal_id: queued.id.clone(),
            rejected_by: "fayeed".into(),
            reason: "that is not a convention, it is a mistake".into(),
        })
        .expect("rejection should succeed");

    assert_eq!(rejected.status, ProposalStatus::Rejected);
    assert!(rejected.memory_id.is_none());
    assert_eq!(
        rejected.rejection_reason.as_deref(),
        Some("that is not a convention, it is a mistake")
    );
    assert!(
        fixture.all_memories().is_empty(),
        "a rejected proposal must leave nothing behind"
    );
}

// --- conflicts -----------------------------------------------------------

#[test]
fn a_contradicting_memory_blocks_approval_until_a_human_chooses() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    fixture.approve(&service, "always use the retry helper for network calls");

    let queued = service
        .propose(proposal(&fixture, "never use the retry helper for network calls"))
        .expect("proposal should queue");
    let outcome = service
        .approve(&ApproveRequest {
            proposal_id: queued.id.clone(),
            approved_by: "fayeed".into(),
            edited_text: None,
            resolutions: Vec::new(),
        })
        .expect("approval should report conflicts rather than fail");

    let ApprovalOutcome::ConflictsRequireChoice { conflicts } = outcome else {
        panic!("expected conflicts, got {outcome:?}")
    };
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].signal, ConflictSignal::OpposingPolarity);
    assert!(
        conflicts[0].explanation.contains("prohibition"),
        "the explanation should say why: {}",
        conflicts[0].explanation
    );

    // Nothing was written while the choice was outstanding.
    assert_eq!(
        fixture.all_memories().len(),
        1,
        "an unresolved conflict must not produce a memory"
    );
    assert_eq!(
        service
            .proposal(&queued.id)
            .expect("proposal should reload")
            .status,
        ProposalStatus::Pending
    );
}

#[test]
fn superseding_the_conflicting_memory_links_and_retires_it() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    let old = fixture.approve(&service, "always use the retry helper for network calls");

    let queued = service
        .propose(proposal(
            &fixture,
            "always use the retry helper for every outbound network call",
        ))
        .expect("proposal should queue");
    let outcome = service
        .approve(&ApproveRequest {
            proposal_id: queued.id,
            approved_by: "fayeed".into(),
            edited_text: None,
            resolutions: vec![ConflictResolution {
                other_memory_id: old.id.clone(),
                choice: ConflictChoice::SupersedeOther,
            }],
        })
        .expect("resolved approval should succeed");

    let ApprovalOutcome::Approved { memory, .. } = outcome else {
        panic!("expected an approval, got {outcome:?}")
    };
    assert_eq!(memory.supersedes.as_deref(), Some(old.id.as_str()));
    let retired = service.get(&old.id).expect("old memory should still exist");
    assert_eq!(
        retired.status,
        MemoryStatus::Superseded,
        "no hard deletes: the replaced memory stays for audit"
    );
}

#[test]
fn keeping_the_existing_memory_rejects_the_proposal() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    let existing = fixture.approve(&service, "always use the retry helper for network calls");

    let queued = service
        .propose(proposal(&fixture, "never use the retry helper for network calls"))
        .expect("proposal should queue");
    let outcome = service
        .approve(&ApproveRequest {
            proposal_id: queued.id.clone(),
            approved_by: "fayeed".into(),
            edited_text: None,
            resolutions: vec![ConflictResolution {
                other_memory_id: existing.id.clone(),
                choice: ConflictChoice::KeepExisting,
            }],
        })
        .expect("the choice should be honoured");

    let ApprovalOutcome::KeptExisting { proposal, kept } = outcome else {
        panic!("expected the existing memory to be kept, got {outcome:?}")
    };
    assert_eq!(proposal.status, ProposalStatus::Rejected);
    assert_eq!(kept, vec![existing.id.clone()]);
    assert_eq!(fixture.all_memories().len(), 1);
    assert_eq!(
        service.get(&existing.id).expect("kept memory").status,
        MemoryStatus::Active
    );
}

#[test]
fn a_resolution_set_that_no_longer_matches_is_refused() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    let queued = service
        .propose(proposal(&fixture, "prefer explicit imports"))
        .expect("proposal should queue");

    // No conflict exists, so resolving one is a sign the caller is working
    // from a stale view.
    let error = service
        .approve(&ApproveRequest {
            proposal_id: queued.id,
            approved_by: "fayeed".into(),
            edited_text: None,
            resolutions: vec![ConflictResolution {
                other_memory_id: "memory-that-does-not-exist".into(),
                choice: ConflictChoice::KeepBoth,
            }],
        })
        .expect_err("a stale conflict set must not be applied");
    assert!(matches!(error, MemoryError::StaleConflictSet), "got {error}");
}

#[test]
fn memories_in_different_repositories_never_conflict() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    fixture.approve(&service, "always use the retry helper for network calls");

    // The same claim, negated, but scoped to a different repository: the two
    // never apply in the same place, so there is nothing to decide.
    let other = fixture.second_repository();
    let mut input = proposal(&fixture, "never use the retry helper for network calls");
    input.repository_id = Some(other);
    let queued = service.propose(input).expect("proposal should queue");

    let outcome = service
        .approve(&ApproveRequest {
            proposal_id: queued.id,
            approved_by: "fayeed".into(),
            edited_text: None,
            resolutions: Vec::new(),
        })
        .expect("disjoint scopes should approve without a decision");
    assert!(
        matches!(outcome, ApprovalOutcome::Approved { .. }),
        "got {outcome:?}"
    );
}

// --- retrieval and usage -------------------------------------------------

#[test]
fn retrieval_selects_by_scope_and_relevance_and_logs_every_injection() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    let relevant = fixture.approve(&service, "run the database migrations before the tests");
    let unrelated = fixture.approve(&service, "the marketing copy lives in a separate repository");

    let retrieved = service
        .for_retrieval(&RetrievalContext {
            project_id: fixture.project_id.clone(),
            repository_id: Some(fixture.repository_id.clone()),
            task_id: Some(fixture.task_id.clone()),
            session_id: None,
            query: "run database migrations".into(),
            surface: "agent_context".into(),
            limit: Some(1),
            record_uses: true,
        })
        .expect("retrieval should work");

    assert_eq!(retrieved.len(), 1);
    assert_eq!(retrieved[0].memory.id, relevant.id);
    assert!(
        retrieved[0].reason.contains("similarity"),
        "a selection should explain itself: {}",
        retrieved[0].reason
    );

    let usage = service.usage(&relevant.id).expect("usage should load");
    assert_eq!(usage.use_count, 1);
    assert_eq!(usage.used_in_tasks, 1);
    assert_eq!(usage.uses[0].surface, "agent_context");
    assert_eq!(usage.uses[0].task_id.as_deref(), Some(fixture.task_id.as_str()));

    let untouched = service.usage(&unrelated.id).expect("usage should load");
    assert_eq!(untouched.use_count, 0);
}

#[test]
fn usage_counts_distinct_tasks_so_the_ui_can_say_used_in_n_tasks() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    let memory = fixture.approve(&service, "run the database migrations before the tests");

    let second_task = fixture.second_task();
    for task_id in [fixture.task_id.clone(), second_task.clone(), second_task] {
        service
            .for_retrieval(&RetrievalContext {
                project_id: fixture.project_id.clone(),
                repository_id: Some(fixture.repository_id.clone()),
                task_id: Some(task_id),
                session_id: None,
                query: "run database migrations".into(),
                surface: "agent_context".into(),
                limit: Some(5),
                record_uses: true,
            })
            .expect("retrieval should work");
    }

    let usage = service.usage(&memory.id).expect("usage should load");
    assert_eq!(usage.use_count, 3, "three injections");
    assert_eq!(usage.used_in_tasks, 2, "across two distinct tasks");
}

#[test]
fn previewing_retrieval_does_not_inflate_the_usage_trail() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    let memory = fixture.approve(&service, "run the database migrations before the tests");

    service
        .for_retrieval(&RetrievalContext {
            project_id: fixture.project_id.clone(),
            repository_id: Some(fixture.repository_id.clone()),
            task_id: Some(fixture.task_id.clone()),
            session_id: None,
            query: "run database migrations".into(),
            surface: "preview".into(),
            limit: Some(5),
            record_uses: false,
        })
        .expect("preview should work");

    assert_eq!(
        service.usage(&memory.id).expect("usage").use_count,
        0,
        "a preview is not a use"
    );
}

#[test]
fn a_narrower_scope_outranks_an_equally_similar_broader_one() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    let text = "run the database migrations before the tests";
    let global = fixture.approve_scoped(&service, text, MemoryScope::Global, "conversation-g");
    let task = fixture.approve_scoped(&service, text, MemoryScope::Task, "conversation-t");

    let retrieved = service
        .for_retrieval(&RetrievalContext {
            project_id: fixture.project_id.clone(),
            repository_id: Some(fixture.repository_id.clone()),
            task_id: Some(fixture.task_id.clone()),
            session_id: None,
            query: "run database migrations".into(),
            surface: "agent_context".into(),
            limit: Some(5),
            record_uses: false,
        })
        .expect("retrieval should work");

    let order: Vec<&str> = retrieved
        .iter()
        .map(|item| item.memory.id.as_str())
        .collect();
    assert_eq!(
        order.first(),
        Some(&task.id.as_str()),
        "the task-scoped memory was written about exactly this work"
    );
    assert!(order.contains(&global.id.as_str()));
}

#[test]
fn revoked_and_superseded_memories_are_never_retrieved() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    let memory = fixture.approve(&service, "run the database migrations before the tests");

    service
        .revoke(&RevokeRequest {
            memory_id: memory.id.clone(),
            revoked_by: "fayeed".into(),
            reason: "the migrations now run in CI".into(),
        })
        .expect("revoke should succeed");

    let retrieved = service
        .for_retrieval(&RetrievalContext {
            project_id: fixture.project_id.clone(),
            repository_id: Some(fixture.repository_id.clone()),
            task_id: Some(fixture.task_id.clone()),
            session_id: None,
            query: "run database migrations".into(),
            surface: "agent_context".into(),
            limit: Some(5),
            record_uses: true,
        })
        .expect("retrieval should work");
    assert!(
        retrieved.is_empty(),
        "a revoked memory must not reach an agent"
    );

    // But it is still there, with its reason, for audit.
    let kept = service.get(&memory.id).expect("revoked memory still exists");
    assert_eq!(kept.status, MemoryStatus::Revoked);
    assert_eq!(kept.revoked_by.as_deref(), Some("fayeed"));
    assert_eq!(
        kept.revocation_reason.as_deref(),
        Some("the migrations now run in CI")
    );
}

// --- revision and audit --------------------------------------------------

#[test]
fn revising_creates_a_new_version_linked_to_the_old_one() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    let original = fixture.approve(&service, "run the formatter before committing");

    let revised = service
        .revise(&ReviseRequest {
            memory_id: original.id.clone(),
            text: "run the formatter and the linter before committing".into(),
            revised_by: "fayeed".into(),
            kind: None,
        })
        .expect("revision should succeed");

    assert_eq!(revised.supersedes.as_deref(), Some(original.id.as_str()));
    assert_eq!(revised.status, MemoryStatus::Active);
    assert_eq!(revised.approved_by, "fayeed");
    // The trace survives the revision.
    assert_eq!(
        revised.origin.conversation_id,
        original.origin.conversation_id
    );

    let previous = service.get(&original.id).expect("old version still exists");
    assert_eq!(previous.status, MemoryStatus::Superseded);
    assert_eq!(previous.text, "run the formatter before committing");
}

#[test]
fn a_superseded_memory_cannot_be_revised_again() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    let original = fixture.approve(&service, "run the formatter before committing");
    service
        .revise(&ReviseRequest {
            memory_id: original.id.clone(),
            text: "run the formatter and the linter before committing".into(),
            revised_by: "fayeed".into(),
            kind: None,
        })
        .expect("first revision should succeed");

    let error = service
        .revise(&ReviseRequest {
            memory_id: original.id,
            text: "something else".into(),
            revised_by: "fayeed".into(),
            kind: None,
        })
        .expect_err("history does not branch");
    assert!(
        matches!(error, MemoryError::MemoryNotActive { .. }),
        "got {error}"
    );
}

#[test]
fn the_audit_trail_records_every_transition() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    let memory = fixture.approve(&service, "run the formatter before committing");
    service
        .revoke(&RevokeRequest {
            memory_id: memory.id.clone(),
            revoked_by: "fayeed".into(),
            reason: "superseded by a pre-commit hook".into(),
        })
        .expect("revoke should succeed");

    let history = service.history(&memory.id).expect("history should load");
    let kinds: Vec<MemoryEventKind> = history.iter().map(|event| event.kind).collect();
    assert!(kinds.contains(&MemoryEventKind::Proposed));
    assert!(kinds.contains(&MemoryEventKind::Approved));
    assert!(kinds.contains(&MemoryEventKind::Revoked));
    for event in &history {
        assert!(!event.actor.is_empty(), "every entry names who did it");
        assert!(
            !event.conversation_id.is_empty(),
            "every entry is anchored to a conversation"
        );
    }
}

// --- listing -------------------------------------------------------------

#[test]
fn listing_shows_active_memories_by_default_and_withdrawn_ones_on_request() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    let kept = fixture.approve(&service, "run the formatter before committing");
    let gone = fixture.approve(&service, "the staging database is shared with QA");
    service
        .revoke(&RevokeRequest {
            memory_id: gone.id.clone(),
            revoked_by: "fayeed".into(),
            reason: "staging is per-developer now".into(),
        })
        .expect("revoke should succeed");

    let active = service
        .list(&MemoryFilters {
            project_id: fixture.project_id.clone(),
            ..MemoryFilters::default()
        })
        .expect("list should work");
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, kept.id);

    let everything = service
        .list(&MemoryFilters {
            project_id: fixture.project_id.clone(),
            statuses: vec![MemoryStatus::Active, MemoryStatus::Revoked],
            ..MemoryFilters::default()
        })
        .expect("list should work");
    assert_eq!(everything.len(), 2, "withdrawn memories remain auditable");
}

#[test]
fn listing_filters_by_kind_and_text() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    fixture.approve(&service, "run the formatter before committing");
    fixture.approve_kind(
        &service,
        "the payment provider rate-limits at 100 requests a minute",
        MemoryKind::Caution,
        "conversation-c",
    );

    let cautions = service
        .list(&MemoryFilters {
            project_id: fixture.project_id.clone(),
            kinds: vec![MemoryKind::Caution],
            ..MemoryFilters::default()
        })
        .expect("list should work");
    assert_eq!(cautions.len(), 1);
    assert_eq!(cautions[0].kind, MemoryKind::Caution);

    let matching = service
        .list(&MemoryFilters {
            project_id: fixture.project_id.clone(),
            contains: Some("formatter".into()),
            ..MemoryFilters::default()
        })
        .expect("list should work");
    assert_eq!(matching.len(), 1);
    assert!(matching[0].text.contains("formatter"));
}

// --- origin verification -------------------------------------------------

#[test]
fn an_unverified_origin_is_labelled_rather_than_assumed_good() {
    let fixture = MemoryFixture::new();
    let service = fixture.service();
    let queued = service
        .propose(proposal(&fixture, "prefer explicit imports"))
        .expect("proposal should queue");
    assert!(
        !queued.origin.verified,
        "with no transcript source the trace is recorded but unconfirmed"
    );
}

#[test]
fn a_transcript_source_verifies_the_excerpt_and_rejects_an_invented_one() {
    let fixture = MemoryFixture::new();
    let session_id = fixture.session_with_message("we always run the formatter before committing");
    let service = MemoryService::with_transcript(
        fixture.storage.clone(),
        Arc::new(mars_brain::AgentSessionTranscript::new(
            fixture.storage.clone(),
        )),
    );

    let mut input = proposal(&fixture, "run the formatter before committing");
    input.conversation_id = session_id.clone();
    input.message_id = "1".into();
    input.quoted_text = "we always run the formatter before committing".into();
    let verified = service.propose(input).expect("a real excerpt should verify");
    assert!(verified.origin.verified);

    let mut invented = proposal(&fixture, "skip the formatter");
    invented.conversation_id = session_id;
    invented.message_id = "1".into();
    invented.quoted_text = "the human said to skip the formatter".into();
    let error = service
        .propose(invented)
        .expect_err("an excerpt nobody said must not be accepted");
    assert!(matches!(error, MemoryError::OriginNotFound { .. }), "got {error}");
}
