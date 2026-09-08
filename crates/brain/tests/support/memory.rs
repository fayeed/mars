//! Fixture for the memory tests.
//!
//! A project with one repository and one task, and no index: memory does not
//! read the code graph, so indexing here would only make the tests slow.

#![allow(dead_code)]

use std::fs;
use std::sync::Arc;

use mars_brain::{
    ApprovalOutcome, ApproveRequest, ConflictChoice, ConflictResolution, Memory, MemoryKind,
    MemoryScope, MemoryService, NewProposal,
};
use mars_core::projects::{OpenRepositoryOutcome, ProjectService};
use mars_security::TrustDecision;
use mars_storage::Storage;
use uuid::Uuid;

use super::TestDirectory;

pub struct MemoryFixture {
    // Declaration order is drop order: the database must close before the
    // directory holding it is removed.
    pub storage: Arc<Storage>,
    pub project_id: String,
    pub repository_id: String,
    pub task_id: String,
    _directory: TestDirectory,
}

impl MemoryFixture {
    pub fn new() -> Self {
        let directory = TestDirectory::new();
        let root = directory.path().join("repo");
        fs::create_dir_all(root.join("src")).expect("repository should exist");
        fs::create_dir(root.join(".git")).expect("Git directory should exist");
        fs::write(root.join("src/main.rs"), "fn main() {}\n").expect("source should write");

        let storage = Arc::new(
            Storage::open(directory.path().join("mars-test.sqlite3")).expect("storage should open"),
        );
        let outcome = ProjectService::new(storage.clone())
            .open_repository(&root, TrustDecision::Trust)
            .expect("repository should open");
        let OpenRepositoryOutcome::Opened(project) = outcome else {
            panic!("trusted repository should open")
        };

        let fixture = Self {
            storage,
            project_id: project.project_id,
            repository_id: project.repository_id,
            task_id: String::new(),
            _directory: directory,
        };
        let task_id = fixture.insert_task("first task");
        Self { task_id, ..fixture }
    }

    pub fn service(&self) -> MemoryService {
        MemoryService::new(self.storage.clone())
    }

    /// Proposes and approves in one step, for tests whose subject is what
    /// happens after a memory already exists.
    pub fn approve(&self, service: &MemoryService, text: &str) -> Memory {
        self.approve_with(
            service,
            text,
            MemoryScope::Repo,
            MemoryKind::Convention,
            text,
        )
    }

    pub fn approve_kind(
        &self,
        service: &MemoryService,
        text: &str,
        kind: MemoryKind,
        conversation: &str,
    ) -> Memory {
        self.approve_with(service, text, MemoryScope::Repo, kind, conversation)
    }

    pub fn approve_scoped(
        &self,
        service: &MemoryService,
        text: &str,
        scope: MemoryScope,
        conversation: &str,
    ) -> Memory {
        self.approve_with(service, text, scope, MemoryKind::Convention, conversation)
    }

    fn approve_with(
        &self,
        service: &MemoryService,
        text: &str,
        scope: MemoryScope,
        kind: MemoryKind,
        conversation: &str,
    ) -> Memory {
        let proposal = service
            .propose(NewProposal {
                project_id: self.project_id.clone(),
                repository_id: Some(self.repository_id.clone()),
                task_id: matches!(scope, MemoryScope::Task).then(|| self.task_id.clone()),
                scope,
                kind,
                text: text.to_owned(),
                confidence: 0.8,
                conversation_id: conversation.to_owned(),
                message_id: format!("message-{}", Uuid::now_v7()),
                quoted_text: format!("the human said: {text}"),
                proposed_by: "claude".into(),
            })
            .expect("fixture proposal should queue");
        let mut request = ApproveRequest {
            proposal_id: proposal.id,
            approved_by: "fayeed".into(),
            edited_text: None,
            resolutions: Vec::new(),
        };
        // Setting up a scenario often means approving memories that overlap on
        // purpose. The fixture answers that the way a person would when both
        // statements are true, and only when actually asked; tests whose
        // subject is the conflict itself call `approve` directly and choose.
        match service
            .approve(&request)
            .expect("fixture approval should succeed")
        {
            ApprovalOutcome::Approved { memory, .. } => return memory,
            ApprovalOutcome::ConflictsRequireChoice { conflicts } => {
                request.resolutions = conflicts
                    .into_iter()
                    .map(|conflict| ConflictResolution {
                        other_memory_id: conflict.other.id,
                        choice: ConflictChoice::KeepBoth,
                    })
                    .collect();
            }
            other => panic!("unexpected fixture approval outcome: {other:?}"),
        }
        match service
            .approve(&request)
            .expect("fixture approval should succeed once resolved")
        {
            ApprovalOutcome::Approved { memory, .. } => memory,
            other => panic!("fixture approval should not need a further decision: {other:?}"),
        }
    }

    pub fn all_memories(&self) -> Vec<String> {
        self.storage
            .with_connection(|connection| {
                let mut statement = connection.prepare("SELECT id FROM memories ORDER BY id")?;
                statement
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect()
            })
            .expect("memories should list")
    }

    pub fn second_task(&self) -> String {
        self.insert_task("second task")
    }

    /// A second repository in the same project, for scope-disjointness tests.
    pub fn second_repository(&self) -> String {
        let id = Uuid::now_v7().to_string();
        self.storage
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO repositories
                        (id, project_id, root_path, git_common_dir, state, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?5)",
                    rusqlite::params![
                        id,
                        self.project_id,
                        format!("/fixture/{id}"),
                        format!("/fixture/{id}/.git"),
                        "2026-01-01T00:00:00Z",
                    ],
                )
            })
            .expect("second repository should insert");
        id
    }

    /// An agent session carrying one event, used as a transcript to verify a
    /// proposal's excerpt against.
    pub fn session_with_message(&self, text: &str) -> String {
        let session_id = Uuid::now_v7().to_string();
        let payload = format!("{{\"text\":\"{text}\"}}");
        self.storage
            .with_transaction(|transaction| {
                transaction.execute(
                    "INSERT INTO agent_sessions
                        (id, task_id, adapter_kind, status, command_json, created_at, started_at)
                     VALUES (?1, ?2, 'claude', 'completed', '{}', ?3, ?3)",
                    rusqlite::params![session_id, self.task_id, "2026-01-01T00:00:00Z"],
                )?;
                transaction.execute(
                    "INSERT INTO agent_events
                        (session_id, sequence, event_type, payload_json, occurred_at)
                     VALUES (?1, 1, 'message', ?2, ?3)",
                    rusqlite::params![session_id, payload, "2026-01-01T00:00:00Z"],
                )?;
                Ok(())
            })
            .expect("fixture session should insert");
        session_id
    }

    fn insert_task(&self, title: &str) -> String {
        let id = Uuid::now_v7().to_string();
        self.storage
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO tasks
                        (id, project_id, repository_id, title, intent, status, created_at,
                         updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, 'ready', ?6, ?6)",
                    rusqlite::params![
                        id,
                        self.project_id,
                        self.repository_id,
                        title,
                        // A real intent, so retrieval relevance has something
                        // to be relevant to.
                        "run the database migrations and fix the failing tests",
                        "2026-01-01T00:00:00Z",
                    ],
                )
            })
            .expect("fixture task should insert");
        id
    }
}
