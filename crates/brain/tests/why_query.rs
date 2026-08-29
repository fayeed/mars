mod support;

use mars_brain::{
    EntityKind, EntityRef, KnowledgeKind, KnowledgeService, NewKnowledge, NewProvenance,
    ProvenanceKind, WhyRequest, WhyService, WhyStatus,
};
use serde_json::json;

#[test]
fn why_traces_code_to_decision_task_session_and_diff_evidence() {
    let fixture = support::indexed_fixture();
    let seeded = Seeded::new(&fixture);
    let knowledge = KnowledgeService::new(fixture.storage.clone());
    let agent = knowledge
        .create_provenance(NewProvenance {
            project_id: seeded.project_id.clone(),
            kind: ProvenanceKind::AgentObserved,
            source: EntityRef::new(EntityKind::AgentSession, "why-session"),
            evidence: json!({ "evidenceIds": ["diff:src/math.ts:5", "event:2"], "summary": "guard added" }),
            observed_at: "2026-08-29T00:10:00Z".into(),
        })
        .expect("agent provenance should persist");
    let human = seeded.human(&knowledge);
    let candidate = knowledge
        .create_candidate(NewKnowledge {
            project_id: seeded.project_id.clone(),
            kind: KnowledgeKind::Decision,
            title: "Reject unsafe sums".into(),
            body: "The add function throws when its result is not a safe integer.".into(),
            confidence: 0.92,
            provenance_id: agent.id,
        })
        .expect("candidate should persist");
    let approved = knowledge
        .approve(&candidate.id, &human.id)
        .expect("human should approve");
    knowledge
        .link(
            &seeded.project_id,
            EntityRef::new(EntityKind::Knowledge, &approved.id),
            "AFFECTS",
            EntityRef::new(EntityKind::Symbol, &seeded.symbol_id),
            &human.id,
            1.0,
        )
        .expect("decision should link to code");

    let answer = WhyService::new(fixture.storage.clone())
        .ask(WhyRequest {
            project_id: seeded.project_id,
            subject: EntityRef::new(EntityKind::Symbol, seeded.symbol_id),
            max_depth: 5,
            max_paths: 10,
        })
        .expect("why answer should build");
    assert_eq!(answer.status, WhyStatus::Answered);
    assert_eq!(answer.facts.len(), 1);
    assert!(answer.inferences.is_empty());
    let kinds = answer.paths[0]
        .steps
        .iter()
        .map(|step| step.kind.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec!["symbol", "knowledge", "agent_session", "task", "evidence"]
    );
    assert!(
        answer
            .sources
            .iter()
            .any(|source| source.location == "diff:src/math.ts:5")
    );
    assert!(
        answer
            .sources
            .iter()
            .all(|source| !source.location.is_empty())
    );
}

#[test]
fn stale_and_conflicting_facts_are_visible_and_insufficient_evidence_is_honest() {
    let fixture = support::indexed_fixture();
    let seeded = Seeded::new(&fixture);
    let knowledge = KnowledgeService::new(fixture.storage.clone());
    let human = seeded.human(&knowledge);
    let why = WhyService::new(fixture.storage.clone());

    let empty = why
        .ask(WhyRequest {
            project_id: seeded.project_id.clone(),
            subject: EntityRef::new(EntityKind::Symbol, seeded.symbol_id.clone()),
            max_depth: 3,
            max_paths: 5,
        })
        .expect("empty answer should not fail");
    assert_eq!(empty.status, WhyStatus::InsufficientEvidence);
    assert!(empty.facts.is_empty());

    let mut ids = Vec::new();
    for (title, body) in [
        ("Throw on overflow", "Unsafe sums throw a RangeError."),
        (
            "Clamp overflow",
            "Unsafe sums are clamped to MAX_SAFE_INTEGER.",
        ),
        ("Old wrapping rule", "Unsafe sums wrap for compatibility."),
    ] {
        let candidate = knowledge
            .create_candidate(NewKnowledge {
                project_id: seeded.project_id.clone(),
                kind: KnowledgeKind::Decision,
                title: title.into(),
                body: body.into(),
                confidence: 1.0,
                provenance_id: human.id.clone(),
            })
            .expect("candidate should persist");
        let approved = knowledge
            .approve(&candidate.id, &human.id)
            .expect("approval should work");
        knowledge
            .link(
                &seeded.project_id,
                EntityRef::new(EntityKind::Knowledge, &approved.id),
                "AFFECTS",
                EntityRef::new(EntityKind::Symbol, &seeded.symbol_id),
                &human.id,
                1.0,
            )
            .expect("fact should link");
        ids.push(approved.id);
    }
    knowledge
        .mark_stale(&ids[2], &human.id)
        .expect("old fact should stale");
    let answer = why
        .ask(WhyRequest {
            project_id: seeded.project_id,
            subject: EntityRef::new(EntityKind::Symbol, seeded.symbol_id),
            max_depth: 4,
            max_paths: 10,
        })
        .expect("conflicting answer should build");
    assert_eq!(answer.status, WhyStatus::AnsweredWithConflicts);
    assert_eq!(answer.facts.len(), 2);
    assert!(
        answer
            .warnings
            .iter()
            .any(|warning| warning.contains("conflicting approved"))
    );
    assert!(
        answer
            .warnings
            .iter()
            .any(|warning| warning.contains("stale"))
    );
}

struct Seeded {
    project_id: String,
    task_id: String,
    symbol_id: String,
}

impl Seeded {
    fn new(fixture: &support::BrainFixture) -> Self {
        fixture
            .storage
            .with_transaction(|transaction| {
                let project_id: String = transaction.query_row(
                    "SELECT project_id FROM repositories WHERE id = ?1",
                    [&fixture.repository_id],
                    |row| row.get(0),
                )?;
                let symbol_id: String = transaction.query_row(
                    "SELECT id FROM symbols WHERE name = 'add' LIMIT 1",
                    [],
                    |row| row.get(0),
                )?;
                transaction.execute(
                    "INSERT INTO tasks (id, project_id, repository_id, title, intent, status, created_at, updated_at)
                     VALUES ('why-task', ?1, ?2, 'Guard add', 'Reject unsafe sums', 'completed', '2026-08-29T00:00:00Z', '2026-08-29T00:10:00Z')",
                    rusqlite::params![project_id, fixture.repository_id],
                )?;
                transaction.execute(
                    "INSERT INTO agent_sessions (id, task_id, adapter_kind, status, command_json, created_at, started_at, ended_at)
                     VALUES ('why-session', 'why-task', 'codex', 'completed', '{}', '2026-08-29T00:00:00Z', '2026-08-29T00:00:00Z', '2026-08-29T00:10:00Z')",
                    [],
                )?;
                Ok(Self { project_id, task_id: "why-task".into(), symbol_id })
            })
            .expect("why fixture should seed")
    }

    fn human(&self, service: &KnowledgeService) -> mars_brain::Provenance {
        service
            .create_provenance(NewProvenance {
                project_id: self.project_id.clone(),
                kind: ProvenanceKind::Human,
                source: EntityRef::new(EntityKind::Task, &self.task_id),
                evidence: json!({ "action": "reviewed in memory UI" }),
                observed_at: "2026-08-29T00:11:00Z".into(),
            })
            .expect("human provenance should persist")
    }
}
