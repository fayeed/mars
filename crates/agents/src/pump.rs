//! Turning a supervised process into an ordered stream of normalized events.
//!
//! The supervisor records every chunk a process writes before it tries to
//! deliver it, and delivery is allowed to fall behind: a reader that is slow
//! loses chunks from the live channel but never from the store. This pump is
//! the reader, so it is the one that has to notice. Every delivered event
//! carries its durable sequence number; a gap means something was dropped in
//! between, and the pump reads the missing chunks back from the store before
//! carrying on. Without that, a dropped chunk would cut a JSON line in half and
//! a dropped exit would leave a run that never ends.
//!
//! The pump also guarantees the ending: exactly one `End` reaches the observer
//! however the process stopped — the provider saying so, the process exiting,
//! someone stopping it, or Mars restarting under it.

use serde_json::Value;

use crate::adapter::{AgentAdapter, JsonLineDecoder, NormalizedAgentEvent};
use crate::process::{LaunchedProcess, ProcessEvent, ProcessEventKind, ProcessSupervisor};

/// Where a run's events go once they are normalized.
pub trait RunObserver: Send {
    /// One event, in order. `stopping` is true once someone asked the run to
    /// stop, so an ending that follows is a stop and not a failure.
    fn event(&mut self, event: NormalizedAgentEvent, stopping: bool);
}

/// Read `launched` to its end, recording each normalized event on the session
/// and handing it to `observer`.
pub async fn pump(
    supervisor: ProcessSupervisor,
    adapter: Box<dyn AgentAdapter>,
    mut launched: LaunchedProcess,
    mut observer: impl RunObserver,
) {
    let mut state = PumpState {
        decoder: JsonLineDecoder::new(),
        last_sequence: 0,
        ended: false,
        stopping: false,
    };
    let session_id = launched.session_id.clone();

    while let Some(event) = launched.events.recv().await {
        if event.sequence > state.last_sequence + 1 {
            backfill(
                &supervisor,
                &session_id,
                adapter.as_ref(),
                &mut state,
                &mut observer,
                Some(event.sequence),
            );
        }
        if event.sequence <= state.last_sequence {
            continue;
        }
        state.handle(
            &supervisor,
            &session_id,
            adapter.as_ref(),
            &event,
            &mut observer,
        );
    }

    // The channel closes when the process is gone. Anything recorded after the
    // last delivered event — the exit itself, often — is read back here.
    backfill(
        &supervisor,
        &session_id,
        adapter.as_ref(),
        &mut state,
        &mut observer,
        None,
    );
    if !state.ended {
        state.emit(
            &supervisor,
            &session_id,
            &mut observer,
            NormalizedAgentEvent::End {
                success: false,
                message: Some("the agent's output ended without saying it was done".into()),
                usage: None,
            },
        );
    }
}

fn backfill(
    supervisor: &ProcessSupervisor,
    session_id: &str,
    adapter: &dyn AgentAdapter,
    state: &mut PumpState,
    observer: &mut impl RunObserver,
    before: Option<i64>,
) {
    let Ok(stored) = supervisor.events(session_id) else {
        return;
    };
    for event in stored {
        if event.sequence <= state.last_sequence {
            continue;
        }
        if before.is_some_and(|before| event.sequence >= before) {
            break;
        }
        state.handle(supervisor, session_id, adapter, &event, observer);
    }
}

struct PumpState {
    decoder: JsonLineDecoder,
    last_sequence: i64,
    ended: bool,
    stopping: bool,
}

impl PumpState {
    fn handle(
        &mut self,
        supervisor: &ProcessSupervisor,
        session_id: &str,
        adapter: &dyn AgentAdapter,
        event: &ProcessEvent,
        observer: &mut impl RunObserver,
    ) {
        self.last_sequence = event.sequence;
        let normalized = match &event.kind {
            ProcessEventKind::Stdout { text } => self.decoder.push(text.as_bytes(), adapter),
            // A CLI's stderr is diagnostics: kept, never mistaken for the run
            // failing. Whether it failed is what the exit and the provider say.
            ProcessEventKind::Stderr { text } if text.trim().is_empty() => Vec::new(),
            ProcessEventKind::Stderr { text } => vec![NormalizedAgentEvent::Unknown {
                provider_type: "stderr".into(),
                raw: Value::String(text.clone()),
            }],
            ProcessEventKind::CancelRequested => {
                self.stopping = true;
                vec![NormalizedAgentEvent::Warning {
                    message: "Stopping the agent".into(),
                }]
            }
            ProcessEventKind::KillEscalated => vec![NormalizedAgentEvent::Warning {
                message: "The agent did not stop when asked, so its process was killed".into(),
            }],
            // Recovered from the store by the gap check; nothing to say.
            ProcessEventKind::DeliveryLagged => Vec::new(),
            ProcessEventKind::Exit { code, .. } => {
                let mut tail = self.decoder.finish(adapter);
                if !self.ended && !tail.iter().any(is_end) {
                    tail.push(if self.stopping {
                        NormalizedAgentEvent::End {
                            success: false,
                            message: Some("Stopped before it finished".into()),
                            usage: None,
                        }
                    } else {
                        adapter.on_exit(*code)
                    });
                }
                tail
            }
            ProcessEventKind::InterruptedOnRestart => vec![NormalizedAgentEvent::End {
                success: false,
                message: Some("Mars restarted while this agent was running".into()),
                usage: None,
            }],
        };
        for event in normalized {
            self.emit(supervisor, session_id, observer, event);
        }
    }

    fn emit(
        &mut self,
        supervisor: &ProcessSupervisor,
        session_id: &str,
        observer: &mut impl RunObserver,
        event: NormalizedAgentEvent,
    ) {
        if is_end(&event) {
            // The first ending is the ending. A provider that reports done and
            // then exits non-zero finished; the exit is not a second verdict.
            if self.ended {
                return;
            }
            self.ended = true;
        }
        let _ = supervisor.record_normalized(session_id, &event);
        observer.event(event, self.stopping);
    }
}

fn is_end(event: &NormalizedAgentEvent) -> bool {
    matches!(event, NormalizedAgentEvent::End { .. })
}
