#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mars_agents::adapter::NormalizedAgentEvent;
use mars_agents::process::{
    ProcessEvent, ProcessEventKind, ProcessSpec, ProcessSupervisor, SessionStatus,
};
use mars_storage::Storage;

#[test]
fn streams_redacted_stdout_and_stderr_in_order_and_captures_nonzero_exit() {
    runtime().block_on(async {
        let fixture = ProcessFixture::new();
        let supervisor = ProcessSupervisor::configured(
            fixture.storage.clone(),
            64,
            Duration::from_millis(100),
            Arc::new(|text| text.replace("secret-token", "[REDACTED]")),
        );
        let mut launched = supervisor
            .launch(shell(
                fixture.path(),
                "printf 'out secret-token'; printf 'err secret-token' >&2; exit 7",
            ))
            .expect("process should launch");

        let delivered = collect_until_exit(&mut launched.events).await;
        let persisted = supervisor
            .events(&launched.session_id)
            .expect("events should persist");
        assert_contiguous(&persisted);
        assert!(persisted.iter().any(|event| matches!(
            &event.kind,
            ProcessEventKind::Stdout { text } if text.contains("out [REDACTED]")
        )));
        assert!(persisted.iter().any(|event| matches!(
            &event.kind,
            ProcessEventKind::Stderr { text } if text.contains("err [REDACTED]")
        )));
        assert!(!format!("{persisted:?}").contains("secret-token"));
        let stored_command: String = fixture
            .storage
            .with_connection(|connection| {
                connection.query_row(
                    "SELECT command_json FROM agent_sessions WHERE id = ?1",
                    [&launched.session_id],
                    |row| row.get(0),
                )
            })
            .expect("stored command should load");
        assert!(!stored_command.contains("secret-token"));
        assert!(stored_command.contains("[REDACTED]"));
        assert!(delivered.iter().any(|event| matches!(
            event.kind,
            ProcessEventKind::Exit {
                code: Some(7),
                success: false
            }
        )));
        let session =
            wait_for_status(&supervisor, &launched.session_id, SessionStatus::Failed).await;
        assert_eq!(session.exit_code, Some(7));
    });
}

#[test]
fn production_supervisor_redacts_secrets_without_custom_configuration() {
    runtime().block_on(async {
        let fixture = ProcessFixture::new();
        let supervisor = ProcessSupervisor::new(fixture.storage.clone());
        let secret = "ghp_abcdefghijklmnopqrstuvwxyz";
        let mut launched = supervisor
            .launch(shell(
                fixture.path(),
                &format!("printf 'GITHUB_TOKEN={secret}\\n'"),
            ))
            .expect("process should launch");
        collect_until_exit(&mut launched.events).await;
        let events = supervisor
            .events(&launched.session_id)
            .expect("events should load");
        let stored_command: String = fixture
            .storage
            .with_connection(|connection| {
                connection.query_row(
                    "SELECT command_json FROM agent_sessions WHERE id = ?1",
                    [&launched.session_id],
                    |row| row.get(0),
                )
            })
            .expect("command should load");
        assert!(!format!("{events:?}{stored_command}").contains(secret));
        assert!(format!("{events:?}{stored_command}").contains("[REDACTED]"));
    });
}

#[test]
fn cancellation_escalates_for_an_uncooperative_process_group() {
    runtime().block_on(async {
        let fixture = ProcessFixture::new();
        let supervisor = ProcessSupervisor::configured(
            fixture.storage.clone(),
            64,
            Duration::from_millis(40),
            Arc::new(str::to_owned),
        );
        let mut launched = supervisor
            .launch(shell(
                fixture.path(),
                "trap '' TERM; printf 'ready\\n'; while :; do sleep 1; done",
            ))
            .expect("process should launch");
        wait_for_output(&mut launched.events, "ready").await;

        supervisor
            .cancel(&launched.session_id)
            .expect("cancellation should signal");
        let delivered = collect_until_exit(&mut launched.events).await;
        assert!(
            delivered
                .iter()
                .any(|event| matches!(event.kind, ProcessEventKind::CancelRequested))
        );
        assert!(
            delivered
                .iter()
                .any(|event| matches!(event.kind, ProcessEventKind::KillEscalated))
        );
        wait_for_status(&supervisor, &launched.session_id, SessionStatus::Cancelled).await;
    });
}

#[test]
fn bounded_delivery_records_backpressure_without_losing_durable_events() {
    runtime().block_on(async {
        let fixture = ProcessFixture::new();
        let supervisor = ProcessSupervisor::configured(
            fixture.storage.clone(),
            1,
            Duration::from_millis(100),
            Arc::new(str::to_owned),
        );
        let launched = supervisor
            .launch(shell(
                fixture.path(),
                "i=0; while [ $i -lt 5000 ]; do printf 'line-%s\\n' \"$i\"; i=$((i+1)); done",
            ))
            .expect("process should launch");

        wait_for_status(&supervisor, &launched.session_id, SessionStatus::Completed).await;
        let events = supervisor
            .events(&launched.session_id)
            .expect("durable events should load");
        assert_contiguous(&events);
        assert!(
            events
                .iter()
                .any(|event| matches!(event.kind, ProcessEventKind::DeliveryLagged))
        );
        assert!(events.iter().any(|event| matches!(
            event.kind,
            ProcessEventKind::Exit {
                code: Some(0),
                success: true
            }
        )));
    });
}

#[test]
fn restart_reconciliation_marks_abandoned_sessions_interrupted() {
    let fixture = ProcessFixture::new();
    fixture
        .storage
        .with_connection(|connection| {
            connection.execute(
                "INSERT INTO agent_sessions
                 (id, adapter_kind, status, command_json, created_at, started_at)
                 VALUES ('abandoned', 'fixture', 'running', '{}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                [],
            )?;
            Ok(())
        })
        .expect("abandoned session should insert");
    let supervisor = ProcessSupervisor::new(fixture.storage.clone());

    assert_eq!(
        supervisor
            .recover_interrupted()
            .expect("recovery should run"),
        vec!["abandoned"]
    );
    assert_eq!(
        supervisor
            .session("abandoned")
            .expect("session should load")
            .status,
        SessionStatus::Interrupted
    );
    assert!(
        supervisor
            .events("abandoned")
            .expect("recovery event should load")
            .iter()
            .any(|event| matches!(event.kind, ProcessEventKind::InterruptedOnRestart))
    );
}

#[test]
fn normalized_provider_events_are_durable_and_ordered_for_handoffs() {
    runtime().block_on(async {
        let fixture = ProcessFixture::new();
        let supervisor = ProcessSupervisor::new(fixture.storage.clone());
        let mut launched = supervisor
            .launch(shell(fixture.path(), "printf done"))
            .expect("process should launch");
        collect_until_exit(&mut launched.events).await;

        assert_eq!(
            supervisor
                .record_normalized(
                    &launched.session_id,
                    &NormalizedAgentEvent::Text {
                        text: "Implemented the safe addition guard.".into(),
                    },
                )
                .expect("normalized text should persist"),
            1
        );
        assert_eq!(
            supervisor
                .record_normalized(
                    &launched.session_id,
                    &NormalizedAgentEvent::Test {
                        command: "pnpm test".into(),
                        status: "passed".into(),
                    },
                )
                .expect("normalized test should persist"),
            2
        );
        let stored: Vec<(i64, String, String)> = fixture
            .storage
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT sequence, event_type, payload_json FROM normalized_agent_events
                     WHERE session_id = ?1 ORDER BY sequence",
                )?;
                statement
                    .query_map([&launched.session_id], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                    })?
                    .collect()
            })
            .expect("normalized events should load");
        assert_eq!(stored.len(), 2);
        assert_eq!(stored[0].1, "text");
        assert_eq!(stored[1].1, "test");
        assert!(stored[1].2.contains("pnpm test"));
    });
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Tokio runtime should build")
}

fn shell(cwd: &Path, script: &str) -> ProcessSpec {
    ProcessSpec::new("fixture", "/bin/sh", ["-c", script], cwd)
}

async fn collect_until_exit(
    receiver: &mut tokio::sync::mpsc::Receiver<ProcessEvent>,
) -> Vec<ProcessEvent> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut events = Vec::new();
    loop {
        let event = tokio::time::timeout_at(deadline, receiver.recv())
            .await
            .expect("process should finish before timeout")
            .expect("event stream should remain open through exit");
        let exited = matches!(event.kind, ProcessEventKind::Exit { .. });
        events.push(event);
        if exited {
            return events;
        }
    }
}

async fn wait_for_output(receiver: &mut tokio::sync::mpsc::Receiver<ProcessEvent>, expected: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let event = tokio::time::timeout_at(deadline, receiver.recv())
            .await
            .expect("output should arrive before timeout")
            .expect("event stream should remain open");
        if matches!(&event.kind, ProcessEventKind::Stdout { text } if text.contains(expected)) {
            return;
        }
    }
}

async fn wait_for_status(
    supervisor: &ProcessSupervisor,
    session_id: &str,
    expected: SessionStatus,
) -> mars_agents::process::SessionRecord {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let record = supervisor.session(session_id).expect("session should load");
        if record.status == expected {
            return record;
        }
        assert!(
            Instant::now() < deadline,
            "session did not reach {expected:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn assert_contiguous(events: &[ProcessEvent]) {
    assert!(!events.is_empty());
    assert!(
        events
            .windows(2)
            .all(|pair| pair[1].sequence == pair[0].sequence + 1)
    );
}

struct ProcessFixture {
    directory: PathBuf,
    storage: Arc<Storage>,
}

impl ProcessFixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow Unix epoch")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "mars-process-fixture-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).expect("fixture directory should exist");
        let storage =
            Arc::new(Storage::open(directory.join("mars.sqlite3")).expect("storage should open"));
        Self { directory, storage }
    }

    fn path(&self) -> &Path {
        &self.directory
    }
}

impl Drop for ProcessFixture {
    fn drop(&mut self) {
        if self.directory.exists() {
            for attempt in 0..20 {
                match fs::remove_dir_all(&self.directory) {
                    Ok(()) => return,
                    Err(_) if attempt < 19 => thread::sleep(Duration::from_millis(10)),
                    Err(error) => panic!("process fixture should be removable: {error}"),
                }
            }
        }
    }
}
