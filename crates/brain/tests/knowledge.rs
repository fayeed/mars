mod support;

use mars_brain::{
    EntityKind, EntityRef, KnowledgeError, KnowledgeKind, KnowledgeService, KnowledgeStatus,
    KnowledgeVia, NewKnowledge, NewProvenance, ProvenanceKind,
};
use serde_json::json;

#[test]
fn candidates_require_bounded_confidence_and_human_resolution() {
    let fixture = support::indexed_fixture();
    let (project_id, task_id) = seed_task(&fixture);
    let service = KnowledgeService::new(fixture.storage.clone());
    let agent = provenance(
        &service,
        &project_id,
        &task_id,
        ProvenanceKind::AgentInferred,
    );
    let human = provenance(&service, &project_id, &task_id, ProvenanceKind::Human);

    for confidence in [-0.01, 1.01, f64::NAN] {
        assert!(matches!(
            service.create_candidate(candidate(&project_id, &agent.id, confidence, "invalid")),
            Err(KnowledgeError::InvalidConfidence(_))
        ));
    }
    let record = service
        .create_candidate(candidate(&project_id, &agent.id, 0.72, "Use stable IDs"))
        .expect("candidate should persist");
    assert_eq!(record.status, KnowledgeStatus::Candidate);
    assert!(matches!(
        service.approve(&record.id, &agent.id),
        Err(KnowledgeError::HumanResolutionRequired)
    ));
    let approved = service
        .approve(&record.id, &human.id)
        .expect("human should approve");
    assert_eq!(approved.status, KnowledgeStatus::Approved);
    assert_eq!(
        approved.resolution_provenance_id.as_deref(),
        Some(human.id.as_str())
    );
    assert!(
        service
            .set_pinned(&approved.id, true, &human.id)
            .expect("approved knowledge should pin")
            .pinned
    );
}

#[test]
fn correction_rejection_staleness_and_superseding_preserve_history() {
    let fixture = support::indexed_fixture();
    let (project_id, task_id) = seed_task(&fixture);
    let service = KnowledgeService::new(fixture.storage.clone());
    let human = provenance(&service, &project_id, &task_id, ProvenanceKind::Human);

    let original = service
        .create_candidate(candidate(&project_id, &human.id, 1.0, "Use mutable IDs"))
        .expect("original should persist");
    let approved = service
        .approve(&original.id, &human.id)
        .expect("approval should work");
    let symbol_id: String = fixture
        .storage
        .with_connection(|connection| {
            connection.query_row("SELECT id FROM symbols ORDER BY id LIMIT 1", [], |row| {
                row.get(0)
            })
        })
        .expect("symbol should exist");
    service
        .link(
            &project_id,
            EntityRef::new(EntityKind::Knowledge, &approved.id),
            "AFFECTS",
            EntityRef::new(EntityKind::Symbol, &symbol_id),
            &human.id,
            0.8,
        )
        .expect("original relationship should persist");
    let correction = service
        .correct(
            &approved.id,
            "Stable entity identity",
            "Use stable IDs and preserve superseded records.",
            &human.id,
        )
        .expect("correction should be transactional");
    assert_eq!(correction.status, KnowledgeStatus::Approved);
    assert_eq!(
        correction.supersedes_id.as_deref(),
        Some(approved.id.as_str())
    );
    assert_eq!(
        service
            .get(&approved.id)
            .expect("old record should remain")
            .status,
        KnowledgeStatus::Superseded
    );
    assert_eq!(service.count(&project_id).expect("records should count"), 2);
    let corrected_link_count: i64 = fixture
        .storage
        .with_connection(|connection| {
            connection.query_row(
                "SELECT COUNT(*) FROM entity_links
                 WHERE source_type = 'knowledge' AND source_id = ?1
                   AND relation = 'AFFECTS' AND target_type = 'symbol' AND target_id = ?2",
                rusqlite::params![correction.id, symbol_id],
                |row| row.get(0),
            )
        })
        .expect("corrected relationship should remain queryable");
    assert_eq!(corrected_link_count, 1);

    let rejected = service
        .create_candidate(candidate(&project_id, &human.id, 0.9, "Delete originals"))
        .and_then(|record| service.reject(&record.id, &human.id))
        .expect("candidate should reject without deletion");
    assert_eq!(rejected.status, KnowledgeStatus::Rejected);
    let stale = service
        .mark_stale(&correction.id, &human.id)
        .expect("approved knowledge should become stale");
    assert_eq!(stale.status, KnowledgeStatus::Stale);
    assert_eq!(
        service.count(&project_id).expect("history should remain"),
        3
    );
}

#[test]
fn generic_links_validate_targets_and_authority_order_is_stable() {
    let fixture = support::indexed_fixture();
    let (project_id, task_id) = seed_task(&fixture);
    let symbol_id: String = fixture
        .storage
        .with_connection(|connection| {
            connection.query_row("SELECT id FROM symbols ORDER BY id LIMIT 1", [], |row| {
                row.get(0)
            })
        })
        .expect("symbol should exist");
    let service = KnowledgeService::new(fixture.storage.clone());
    let human = provenance(&service, &project_id, &task_id, ProvenanceKind::Human);
    let git = provenance(
        &service,
        &project_id,
        &task_id,
        ProvenanceKind::ExtractedGit,
    );
    let inferred = provenance(
        &service,
        &project_id,
        &task_id,
        ProvenanceKind::AgentInferred,
    );

    let mut records = Vec::new();
    for (source, confidence, title) in [
        (&inferred, 0.99, "Agent inference"),
        (&human, 0.51, "Human decision"),
        (&git, 0.95, "Git observation"),
    ] {
        let candidate = service
            .create_candidate(candidate(&project_id, &source.id, confidence, title))
            .expect("candidate should persist");
        records.push(
            service
                .approve(&candidate.id, &human.id)
                .expect("approval should work"),
        );
    }
    let ranked = service
        .list(&project_id, true)
        .expect("knowledge should rank");
    assert_eq!(
        ranked
            .iter()
            .map(|record| record.title.as_str())
            .collect::<Vec<_>>(),
        vec!["Human decision", "Git observation", "Agent inference"]
    );

    let decision = &records[0];
    let link = service
        .link(
            &project_id,
            EntityRef::new(EntityKind::Knowledge, &decision.id),
            "AFFECTS",
            EntityRef::new(EntityKind::Symbol, &symbol_id),
            &human.id,
            0.8,
        )
        .expect("valid generic link should persist");
    assert_eq!(link.relation, "AFFECTS");
    assert!(matches!(
        service.link(
            &project_id,
            EntityRef::new(EntityKind::Task, "missing-task"),
            "ABOUT",
            EntityRef::new(EntityKind::Knowledge, &decision.id),
            &human.id,
            1.0,
        ),
        Err(KnowledgeError::EntityNotFound(_))
    ));
}

#[test]
fn a_record_says_which_task_and_agent_it_came_from_and_keeps_that_through_a_correction() {
    let fixture = support::indexed_fixture();
    let (project_id, task_id) = seed_task(&fixture);
    let service = KnowledgeService::new(fixture.storage.clone());

    // Recorded by an agent over MCP, while it worked.
    let recorded = service
        .create_provenance(NewProvenance {
            project_id: project_id.clone(),
            kind: ProvenanceKind::AgentInferred,
            source: EntityRef::new(EntityKind::Task, &task_id),
            evidence: json!({
                "via": "mcp",
                "agent": "claude",
                "runId": "run-1",
                "alternatives": ["fixed window", "leaky bucket"],
                "files": ["src/not-indexed.ts"],
            }),
            observed_at: "2026-08-29T00:00:00Z".into(),
        })
        .expect("provenance");
    let decision = service
        .create_candidate(candidate(
            &project_id,
            &recorded.id,
            0.8,
            "Use a token bucket",
        ))
        .expect("candidate");
    let origin = service.origin(&decision).expect("origin");
    assert_eq!(origin.via, KnowledgeVia::Agent);
    assert_eq!(origin.task_id.as_deref(), Some(task_id.as_str()));
    assert_eq!(origin.agent.as_deref(), Some("claude"));
    assert_eq!(origin.run_id.as_deref(), Some("run-1"));
    assert_eq!(origin.alternatives, ["fixed window", "leaky bucket"]);
    assert_eq!(origin.files, ["src/not-indexed.ts"]);

    // Read out of a finished session: the session names the task and agent.
    fixture
        .storage
        .with_connection(|connection| {
            connection.execute(
                "INSERT INTO agent_sessions (id, task_id, adapter_kind, status, command_json, created_at, started_at)
                 VALUES ('session-1', ?1, 'codex', 'completed', '{}', '2026-08-29T00:00:00Z', '2026-08-29T00:00:00Z')",
                [&task_id],
            )
        })
        .expect("session");
    let extracted = service
        .create_provenance(NewProvenance {
            project_id: project_id.clone(),
            kind: ProvenanceKind::AgentInferred,
            source: EntityRef::new(EntityKind::AgentSession, "session-1"),
            evidence: json!({ "evidenceIds": [], "category": "decision" }),
            observed_at: "2026-08-29T00:00:00Z".into(),
        })
        .expect("provenance");
    let found = service
        .create_candidate(candidate(&project_id, &extracted.id, 0.6, "Keep SQLite"))
        .expect("candidate");
    let origin = service.origin(&found).expect("origin");
    assert_eq!(origin.via, KnowledgeVia::Extraction);
    assert_eq!(origin.task_id.as_deref(), Some(task_id.as_str()));
    assert_eq!(origin.agent.as_deref(), Some("codex"));

    // Corrected by a person: theirs now, but still from that task and agent.
    let human = provenance(&service, &project_id, &task_id, ProvenanceKind::Human);
    service.approve(&decision.id, &human.id).expect("approve");
    let corrected = service
        .correct(
            &decision.id,
            "Use a token bucket per tenant",
            "Per tenant.",
            &human.id,
        )
        .expect("correct");
    let origin = service.origin(&corrected).expect("origin");
    assert_eq!(origin.via, KnowledgeVia::Person);
    assert_eq!(origin.task_id.as_deref(), Some(task_id.as_str()));
    assert_eq!(origin.agent.as_deref(), Some("claude"));
    assert_eq!(origin.alternatives, ["fixed window", "leaky bucket"]);
}

fn provenance(
    service: &KnowledgeService,
    project_id: &str,
    task_id: &str,
    kind: ProvenanceKind,
) -> mars_brain::Provenance {
    service
        .create_provenance(NewProvenance {
            project_id: project_id.into(),
            kind,
            source: EntityRef::new(EntityKind::Task, task_id),
            evidence: json!({ "taskId": task_id, "visible": true }),
            observed_at: "2026-08-29T00:00:00Z".into(),
        })
        .expect("provenance should persist")
}

fn candidate(project_id: &str, provenance_id: &str, confidence: f64, title: &str) -> NewKnowledge {
    NewKnowledge {
        project_id: project_id.into(),
        kind: KnowledgeKind::Decision,
        title: title.into(),
        body: format!("Rationale for {title}."),
        confidence,
        provenance_id: provenance_id.into(),
    }
}

fn seed_task(fixture: &support::BrainFixture) -> (String, String) {
    fixture
        .storage
        .with_connection(|connection| {
            let project_id: String = connection.query_row(
                "SELECT project_id FROM repositories WHERE id = ?1",
                [&fixture.repository_id],
                |row| row.get(0),
            )?;
            connection.execute(
                "INSERT INTO tasks (id, project_id, repository_id, title, intent, status, created_at, updated_at)
                 VALUES ('knowledge-task', ?1, ?2, 'Knowledge task', 'Preserve rationale', 'ready', '2026-08-29T00:00:00Z', '2026-08-29T00:00:00Z')",
                rusqlite::params![project_id, fixture.repository_id],
            )?;
            Ok((project_id, "knowledge-task".to_owned()))
        })
        .expect("task should seed")
}
