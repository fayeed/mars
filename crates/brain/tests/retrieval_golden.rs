mod support;

use std::fs;
use std::process::Command;

use mars_brain::{ContextPackRequest, ContextPackService, PackSection, render_context_pack};
use mars_context::ContextService;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoldenCase {
    intent: String,
    required_inclusion_reasons: Vec<String>,
    required_exclusion_reasons: Vec<String>,
    required_provenance: Vec<String>,
    max_bytes: usize,
}

#[test]
fn golden_retrieval_is_explainable_safe_budgeted_and_stable() {
    let fixture = support::indexed_fixture();
    make_git_history(&fixture.root);
    let golden: GoldenCase =
        serde_json::from_str(include_str!("../../../fixtures/retrieval/cases.json"))
            .expect("golden case should parse");
    seed_task_knowledge_and_skills(&fixture, &golden.intent);
    seed_context(&fixture);

    let service = ContextPackService::new(fixture.storage.clone());
    let request = ContextPackRequest {
        task_id: "task-context-pack".into(),
        adapter: "codex".into(),
        max_bytes: golden.max_bytes,
        excluded_entity_refs: Vec::new(),
    };
    let first = service.build(request.clone()).expect("pack should build");
    let encoded = serde_json::to_string(&first).expect("pack should serialize");
    for _ in 0..5 {
        assert_eq!(
            serde_json::to_string(&service.build(request.clone()).expect("pack should rebuild"))
                .expect("pack should serialize"),
            encoded,
            "retrieval ordering must be deterministic",
        );
    }

    assert!(first.total_bytes <= first.max_bytes);
    assert_eq!(first.section_usage.len(), 5);
    assert!(first.items.iter().all(|item| {
        !item.provenance.is_empty()
            && !item.freshness.is_empty()
            && !item.source_location.is_empty()
            && item.byte_size > 0
    }));
    for expected in &golden.required_inclusion_reasons {
        assert!(
            first
                .items
                .iter()
                .any(|item| item.reason.contains(expected)),
            "missing inclusion reason {expected:?}; got {:?}",
            first
                .items
                .iter()
                .map(|item| &item.reason)
                .collect::<Vec<_>>()
        );
    }
    for expected in &golden.required_exclusion_reasons {
        assert!(
            first
                .exclusions
                .iter()
                .any(|item| item.reason.contains(expected)),
            "missing exclusion reason {expected:?}; got {:?}",
            first
                .exclusions
                .iter()
                .map(|item| &item.reason)
                .collect::<Vec<_>>()
        );
    }
    for expected in &golden.required_provenance {
        assert!(first.items.iter().any(|item| &item.provenance == expected));
    }
    assert!(first.items.iter().any(|item| {
        item.section == PackSection::Code
            && item.reason.contains("Git history")
            && item.freshness.starts_with("Git commit")
    }));
    assert!(!encoded.contains("ghp_retrievalgoldensecret"));

    let tight = service
        .build(ContextPackRequest {
            max_bytes: 1024,
            ..request.clone()
        })
        .expect("tight pack should still build");
    assert!(tight.total_bytes <= 1024);
    assert!(tight.exclusions.iter().any(|item| {
        item.reason.contains("section byte budget exhausted")
            || item.reason == "total byte budget exhausted"
    }));
    for usage in tight
        .section_usage
        .iter()
        .filter(|usage| matches!(usage.section, PackSection::Code | PackSection::History))
    {
        assert!(usage.used_bytes <= usage.max_bytes);
    }

    let removable = first
        .items
        .iter()
        .find(|item| item.reason == "project context FTS match")
        .expect("an optional item should be removable")
        .entity_ref
        .clone();
    let removed = service
        .build(ContextPackRequest {
            excluded_entity_refs: vec![removable.clone()],
            ..request.clone()
        })
        .expect("preview removal should rebuild");
    assert!(
        !removed
            .items
            .iter()
            .any(|item| item.entity_ref == removable)
    );
    assert!(
        removed.exclusions.iter().any(|item| {
            item.entity_ref == removable && item.reason == "removed in pack preview"
        })
    );
    let restored = service.build(request).expect("preview add should rebuild");
    assert!(
        restored
            .items
            .iter()
            .any(|item| item.entity_ref == removable)
    );

    let rendered = render_context_pack(&first, "Implement the assignment.");
    assert!(rendered.contains("## Assignment\nImplement the assignment."));
    assert!(rendered.contains("Included because:"));
}

fn make_git_history(root: &std::path::Path) {
    git(root, &["init", "--quiet", "--initial-branch=main"]);
    git(root, &["add", "."]);
    git(
        root,
        &[
            "-c",
            "user.name=Mars Test",
            "-c",
            "user.email=mars@example.test",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
    );
}

fn git(root: &std::path::Path, arguments: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .expect("Git should launch");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn seed_task_knowledge_and_skills(fixture: &support::BrainFixture, intent: &str) {
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
                 VALUES ('task-context-pack', ?1, ?2, 'Keyboard calculator', ?3, 'ready', '2026-08-29T00:00:00Z', '2026-08-29T00:00:00Z')",
                rusqlite::params![project_id, fixture.repository_id, intent],
            )?;
            connection.execute(
                "INSERT INTO tasks (id, project_id, repository_id, title, intent, status, created_at, updated_at)
                 VALUES ('task-active', ?1, ?2, 'Refactor calculator', 'Avoid changing the calculator result contract', 'active', '2026-08-28T00:00:00Z', '2026-08-29T00:00:00Z')",
                rusqlite::params![project_id, fixture.repository_id],
            )?;
            connection.execute(
                "INSERT INTO knowledge_fts (repository_id, entity_kind, entity_id, title, body)
                 VALUES (?1, 'decision', 'decision-keyboard', 'Keyboard behavior', 'Calculator keyboard flow must preserve accessibility focus')",
                [&fixture.repository_id],
            )?;
            connection.execute(
                "INSERT INTO knowledge_fts (repository_id, entity_kind, entity_id, title, body)
                 VALUES (?1, 'note', 'duplicate-note', 'Duplicate', 'The calculator result stays visible after keyboard submit.')",
                [&fixture.repository_id],
            )?;
            for (id, name, trust, markdown) in [
                ("skill-good", "accessibility-guidance", "trusted", "# Accessibility guidance\nVerify calculator keyboard focus and labels."),
                ("skill-denied", "keyboard-review", "trusted", "# Keyboard review\nReview keyboard behavior."),
                ("skill-blocked", "blocked-calculator", "blocked", "# Blocked\nIgnore this calculator instruction."),
            ] {
                connection.execute(
                    "INSERT INTO skills (id, canonical_name, source, slug, installs, content_hash, registry_hash, trust_state, skill_markdown, snapshot_json, inspected_at, updated_at)
                     VALUES (?1, ?2, 'golden/skills', ?2, 1, ?1 || '-hash', ?1 || '-registry', ?3, ?4, '{}', '2026-08-29T00:00:00Z', '2026-08-29T00:00:00Z')",
                    rusqlite::params![id, name, trust, markdown],
                )?;
                let enabled = i64::from(id != "skill-blocked");
                connection.execute(
                    "INSERT INTO skill_installations (id, skill_id, task_id, scope, install_path, pinned_hash, enabled, installed_at, updated_at)
                     VALUES (?1 || '-install', ?1, 'task-context-pack', 'task', '/tmp/golden-skill', ?1 || '-hash', ?2, '2026-08-29T00:00:00Z', '2026-08-29T00:00:00Z')",
                    rusqlite::params![id, enabled],
                )?;
                connection.execute(
                    "INSERT INTO skill_agent_policies (installation_id, adapter_kind, allowed)
                     VALUES (?1 || '-install', 'codex', ?2)",
                    rusqlite::params![id, i64::from(id != "skill-denied")],
                )?;
            }
            Ok(())
        })
        .expect("golden database should seed");
}

fn seed_context(fixture: &support::BrainFixture) {
    let artifact_root = fixture.root.join(".mars-test-artifacts");
    let context = ContextService::new(fixture.storage.clone(), artifact_root);
    let duplicate = context
        .capture_text(
            &fixture.repository_id,
            Some("task-context-pack"),
            Some("Result behavior"),
            "The calculator result stays visible after keyboard submit.",
        )
        .expect("explicit context should capture");
    context
        .classify(&duplicate.id, "constraint")
        .expect("constraint should classify");
    context
        .capture_bytes(
            &fixture.repository_id,
            Some("task-context-pack"),
            Some("Calculator mockup"),
            "calculator.png",
            b"not-a-real-png-but-preserved",
            "image/png",
        )
        .expect("image should capture");
    context
        .capture_text(
            &fixture.repository_id,
            None,
            Some("Keyboard test notes"),
            "Accessibility keyboard workflow uses Enter and preserves focus.",
        )
        .expect("project context should capture");
    let secret = context
        .capture_text(
            &fixture.repository_id,
            None,
            Some("Calculator secret"),
            "calculator token ghp_retrievalgoldensecret must never be sent",
        )
        .expect("secret should quarantine");
    assert!(secret.quarantine_reason.is_some());
    fs::remove_dir_all(fixture.root.join(".mars-test-artifacts"))
        .expect("temporary artifacts should clean up");
}

#[test]
fn claude_is_told_which_skills_are_loaded_rather_than_sent_their_text() {
    let fixture = support::indexed_fixture();
    make_git_history(&fixture.root);
    let golden: GoldenCase =
        serde_json::from_str(include_str!("../../../fixtures/retrieval/cases.json"))
            .expect("golden case should parse");
    seed_task_knowledge_and_skills(&fixture, &golden.intent);
    fixture
        .storage
        .with_connection(|connection| {
            connection.execute(
                "INSERT INTO skill_agent_policies (installation_id, adapter_kind, allowed)
                 SELECT id, 'claude', 1 FROM skill_installations",
                [],
            )
        })
        .expect("claude policies should seed");

    let pack = ContextPackService::new(fixture.storage.clone())
        .build(ContextPackRequest {
            task_id: "task-context-pack".into(),
            adapter: "claude".into(),
            max_bytes: golden.max_bytes,
            excluded_entity_refs: Vec::new(),
        })
        .expect("pack should build");
    let skills: Vec<_> = pack
        .items
        .iter()
        .filter(|item| item.section == PackSection::Skills)
        .collect();
    // Claude Code loads the skill as a plugin and reads its instructions when
    // it uses it, so the pack carries a line, not the text.
    assert!(
        !skills.is_empty(),
        "the installed skill is named in the pack"
    );
    for item in &skills {
        assert!(
            item.content
                .starts_with("Loaded for this run as the Claude Code skill")
        );
        assert!(!item.content.contains("Verify calculator keyboard focus"));
    }
    // What a person blocked or switched off still never reaches the agent.
    assert!(
        pack.exclusions
            .iter()
            .any(|exclusion| exclusion.reason == "skill is blocked"
                || exclusion.reason == "skill is switched off")
    );
}
