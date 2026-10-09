use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use mars_storage::{Storage, StorageError};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep, timeout};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::adapter::{AdapterCommand, NormalizedAgentEvent};

const RAW_EVENT_CAPACITY: usize = 64;
const STREAM_CHUNK_BYTES: usize = 4 * 1024;

pub type Redactor = Arc<dyn Fn(&str) -> String + Send + Sync + 'static>;

#[derive(Clone, Debug)]
pub struct ProcessSpec {
    pub adapter_kind: String,
    pub task_id: Option<String>,
    pub worktree_id: Option<String>,
    pub program: PathBuf,
    pub arguments: Vec<OsString>,
    pub cwd: PathBuf,
    pub environment: Vec<(OsString, OsString)>,
    /// Variables the child must not inherit from Mars, such as an API key the
    /// person chose not to bill. Recorded by name only.
    pub removed_environment: Vec<OsString>,
    /// Written to the child's standard input, which is then closed. Never
    /// persisted on the session: it is the prompt, and the run keeps its own
    /// copy where a reader can find it.
    pub stdin: Option<String>,
}

impl ProcessSpec {
    /// A process for exactly what an adapter built: its program, arguments,
    /// working directory, standard input and added environment.
    #[must_use]
    pub fn from_command(adapter_kind: impl Into<String>, command: AdapterCommand) -> Self {
        let mut spec = Self::new(
            adapter_kind,
            &command.program,
            command.arguments,
            &command.cwd,
        );
        spec.environment = command.environment;
        spec.stdin = command.stdin;
        spec
    }

    #[must_use]
    pub fn new(
        adapter_kind: impl Into<String>,
        program: impl AsRef<Path>,
        arguments: impl IntoIterator<Item = impl Into<OsString>>,
        cwd: impl AsRef<Path>,
    ) -> Self {
        Self {
            adapter_kind: adapter_kind.into(),
            task_id: None,
            worktree_id: None,
            program: program.as_ref().to_owned(),
            arguments: arguments.into_iter().map(Into::into).collect(),
            cwd: cwd.as_ref().to_owned(),
            environment: Vec::new(),
            removed_environment: Vec::new(),
            stdin: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ProcessEventKind {
    Stdout { text: String },
    Stderr { text: String },
    CancelRequested,
    KillEscalated,
    DeliveryLagged,
    Exit { code: Option<i32>, success: bool },
    InterruptedOnRestart,
}

impl ProcessEventKind {
    fn event_type(&self) -> &'static str {
        match self {
            Self::Stdout { .. } => "stdout",
            Self::Stderr { .. } => "stderr",
            Self::CancelRequested => "cancel_requested",
            Self::KillEscalated => "kill_escalated",
            Self::DeliveryLagged => "delivery_lagged",
            Self::Exit { .. } => "exit",
            Self::InterruptedOnRestart => "interrupted_on_restart",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessEvent {
    pub session_id: String,
    pub sequence: i64,
    pub kind: ProcessEventKind,
    pub occurred_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Running,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

impl SessionStatus {
    fn parse(value: &str) -> Result<Self, ProcessError> {
        match value {
            "running" => Ok(Self::Running),
            "cancelling" => Ok(Self::Cancelling),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "interrupted" => Ok(Self::Interrupted),
            other => Err(ProcessError::InvalidStoredStatus(other.to_owned())),
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Cancelling => "cancelling",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecord {
    pub id: String,
    pub status: SessionStatus,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
}

pub struct LaunchedProcess {
    pub session_id: String,
    pub events: mpsc::Receiver<ProcessEvent>,
}

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("process I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("session `{0}` does not exist")]
    SessionNotFound(String),
    #[error("session `{0}` is not running")]
    SessionNotRunning(String),
    #[error("process state lock was poisoned")]
    StatePoisoned,
    #[error("spawned process did not expose its {0} stream")]
    MissingStream(&'static str),
    #[error("stored session status is invalid: {0}")]
    InvalidStoredStatus(String),
    #[error("failed to encode session data: {0}")]
    Json(#[from] serde_json::Error),
    #[error("failed to format a timestamp: {0}")]
    Timestamp(#[from] time::error::Format),
}

struct SupervisorInner {
    storage: Arc<Storage>,
    running: Mutex<HashMap<String, CancellationToken>>,
    delivery_capacity: usize,
    cancellation_grace: Duration,
    redactor: Redactor,
}

#[derive(Clone)]
pub struct ProcessSupervisor {
    inner: Arc<SupervisorInner>,
}

impl ProcessSupervisor {
    #[must_use]
    pub fn new(storage: Arc<Storage>) -> Self {
        Self::configured(
            storage,
            256,
            Duration::from_secs(2),
            Arc::new(mars_security::redact),
        )
    }

    #[must_use]
    pub fn configured(
        storage: Arc<Storage>,
        delivery_capacity: usize,
        cancellation_grace: Duration,
        redactor: Redactor,
    ) -> Self {
        Self {
            inner: Arc::new(SupervisorInner {
                storage,
                running: Mutex::new(HashMap::new()),
                delivery_capacity: delivery_capacity.max(1),
                cancellation_grace,
                redactor,
            }),
        }
    }

    pub fn launch(&self, spec: ProcessSpec) -> Result<LaunchedProcess, ProcessError> {
        let session_id = Uuid::now_v7().to_string();
        let mut command = Command::new(&spec.program);
        command
            .args(&spec.arguments)
            .current_dir(&spec.cwd)
            .envs(spec.environment.iter().cloned())
            .stdin(if spec.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for variable in &spec.removed_environment {
            command.env_remove(variable);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.as_std_mut().process_group(0);
        }
        let mut child = command.spawn()?;
        let pid = child.id();
        if let (Some(input), Some(mut stdin)) = (spec.stdin.clone(), child.stdin.take()) {
            // Written on its own task so a child that reads slowly cannot hold
            // up the reads of its output; dropping the handle closes the pipe,
            // which is how the child knows the prompt is complete.
            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt;
                let _ = stdin.write_all(input.as_bytes()).await;
                let _ = stdin.shutdown().await;
            });
        }
        let stdout = child
            .stdout
            .take()
            .ok_or(ProcessError::MissingStream("stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or(ProcessError::MissingStream("stderr"))?;
        if let Err(error) = insert_session(
            &self.inner.storage,
            &session_id,
            &spec,
            pid,
            &self.inner.redactor,
        ) {
            let _ = child.start_kill();
            return Err(error);
        }

        let cancellation = CancellationToken::new();
        self.running()?
            .insert(session_id.clone(), cancellation.clone());
        let (raw_tx, raw_rx) = mpsc::channel(RAW_EVENT_CAPACITY);
        let (delivery_tx, delivery_rx) = mpsc::channel(self.inner.delivery_capacity);
        tokio::spawn(read_stream(stdout, RawKind::Stdout, raw_tx.clone()));
        tokio::spawn(read_stream(stderr, RawKind::Stderr, raw_tx));
        tokio::spawn(run_process(
            self.inner.clone(),
            session_id.clone(),
            child,
            pid,
            cancellation,
            raw_rx,
            delivery_tx,
        ));

        Ok(LaunchedProcess {
            session_id,
            events: delivery_rx,
        })
    }

    pub fn cancel(&self, session_id: &str) -> Result<(), ProcessError> {
        let token = self
            .running()?
            .get(session_id)
            .cloned()
            .ok_or_else(|| ProcessError::SessionNotRunning(session_id.to_owned()))?;
        token.cancel();
        Ok(())
    }

    pub fn session(&self, session_id: &str) -> Result<SessionRecord, ProcessError> {
        load_session(&self.inner.storage, session_id)
    }

    pub fn events(&self, session_id: &str) -> Result<Vec<ProcessEvent>, ProcessError> {
        load_events(&self.inner.storage, session_id)
    }

    pub fn record_normalized(
        &self,
        session_id: &str,
        event: &NormalizedAgentEvent,
    ) -> Result<i64, ProcessError> {
        let occurred_at = now()?;
        let payload = serde_json::to_string(event)?;
        let event_type = serde_json::to_value(event)?
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        let sequence = self.inner.storage.with_transaction(|transaction| {
            let exists: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM agent_sessions WHERE id = ?1",
                [session_id],
                |row| row.get(0),
            )?;
            if exists == 0 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            let sequence = transaction.query_row(
                "SELECT COALESCE(MAX(sequence), 0) + 1 FROM normalized_agent_events WHERE session_id = ?1",
                [session_id],
                |row| row.get::<_, i64>(0),
            )?;
            transaction.execute(
                "INSERT INTO normalized_agent_events
                 (session_id, sequence, event_type, payload_json, occurred_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![session_id, sequence, event_type, payload, occurred_at],
            )?;
            Ok(sequence)
        })?;
        Ok(sequence)
    }

    pub fn recover_interrupted(&self) -> Result<Vec<String>, ProcessError> {
        let ids: Vec<String> = self.inner.storage.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id FROM agent_sessions WHERE status IN ('running', 'cancelling') ORDER BY created_at, id",
            )?;
            statement.query_map([], |row| row.get(0))?.collect()
        })?;
        for id in &ids {
            update_session(
                &self.inner.storage,
                id,
                SessionStatus::Interrupted,
                None,
                Some("desktop restarted while the process was active"),
            )?;
            append_event(
                &self.inner.storage,
                id,
                &ProcessEventKind::InterruptedOnRestart,
            )?;
        }
        Ok(ids)
    }

    fn running(&self) -> Result<MutexGuard<'_, HashMap<String, CancellationToken>>, ProcessError> {
        self.inner
            .running
            .lock()
            .map_err(|_| ProcessError::StatePoisoned)
    }
}

#[derive(Clone, Copy)]
enum RawKind {
    Stdout,
    Stderr,
}

struct RawEvent {
    kind: RawKind,
    text: String,
}

async fn read_stream(
    mut stream: impl AsyncRead + Unpin,
    kind: RawKind,
    sender: mpsc::Sender<RawEvent>,
) {
    let mut buffer = vec![0_u8; STREAM_CHUNK_BYTES];
    let mut decoder = Utf8Chunks::default();
    loop {
        let text = match stream.read(&mut buffer).await {
            Ok(0) | Err(_) => {
                let rest = decoder.finish();
                if !rest.is_empty() {
                    let _ = sender.send(RawEvent { kind, text: rest }).await;
                }
                break;
            }
            Ok(read) => decoder.push(&buffer[..read]),
        };
        if text.is_empty() {
            continue;
        }
        if sender.send(RawEvent { kind, text }).await.is_err() {
            break;
        }
    }
}

/// Decodes a byte stream read in arbitrary chunks.
///
/// A read can end partway through a multi-byte character. Decoding each chunk
/// on its own turns both halves into replacement characters, which in an agent
/// stream corrupts the JSON line the character sat in. The incomplete tail is
/// held back until the bytes that finish it arrive.
#[derive(Default)]
pub struct Utf8Chunks {
    pending: Vec<u8>,
}

impl Utf8Chunks {
    /// Everything decodable so far.
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut text = String::new();
        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(valid) => {
                    text.push_str(valid);
                    self.pending.clear();
                    return text;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    text.push_str(std::str::from_utf8(&self.pending[..valid]).unwrap_or_default());
                    match error.error_len() {
                        // Bytes that are invalid wherever they end up: replace
                        // them and carry on with the rest of the chunk.
                        Some(invalid) => {
                            text.push(char::REPLACEMENT_CHARACTER);
                            self.pending.drain(..valid + invalid);
                        }
                        // A character cut off by the end of the read.
                        None => {
                            self.pending.drain(..valid);
                            return text;
                        }
                    }
                }
            }
        }
    }

    /// Whatever is left when the stream ends, which can only be a character
    /// that never finished.
    pub fn finish(&mut self) -> String {
        let rest = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        rest
    }
}

async fn run_process(
    inner: Arc<SupervisorInner>,
    session_id: String,
    mut child: Child,
    pid: Option<u32>,
    cancellation: CancellationToken,
    mut raw: mpsc::Receiver<RawEvent>,
    delivery: mpsc::Sender<ProcessEvent>,
) {
    let mut cancelling = false;
    let mut escalated = false;
    let mut deadline: Option<Instant> = None;
    let mut delivery_lagged = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Err(error) => break Err(error),
            Ok(None) => {}
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) && !escalated {
            escalated = true;
            terminate_process(pid, true, &mut child);
            let _ = record_and_deliver(
                &inner,
                &session_id,
                ProcessEventKind::KillEscalated,
                &delivery,
                &mut delivery_lagged,
            );
        }
        while let Ok(event) = raw.try_recv() {
            let text = (inner.redactor)(&event.text);
            let kind = match event.kind {
                RawKind::Stdout => ProcessEventKind::Stdout { text },
                RawKind::Stderr => ProcessEventKind::Stderr { text },
            };
            let _ = record_and_deliver(&inner, &session_id, kind, &delivery, &mut delivery_lagged);
        }
        if cancellation.is_cancelled() && !cancelling {
            cancelling = true;
            deadline = Some(Instant::now() + inner.cancellation_grace);
            let _ = set_cancelling(&inner.storage, &session_id);
            let _ = record_and_deliver(
                &inner,
                &session_id,
                ProcessEventKind::CancelRequested,
                &delivery,
                &mut delivery_lagged,
            );
            terminate_process(pid, false, &mut child);
        }
        sleep(Duration::from_millis(5)).await;
    };

    while let Ok(Some(event)) = timeout(Duration::from_millis(25), raw.recv()).await {
        let text = (inner.redactor)(&event.text);
        let kind = match event.kind {
            RawKind::Stdout => ProcessEventKind::Stdout { text },
            RawKind::Stderr => ProcessEventKind::Stderr { text },
        };
        let _ = record_and_deliver(&inner, &session_id, kind, &delivery, &mut delivery_lagged);
    }

    match status {
        Ok(status) => {
            let kind = ProcessEventKind::Exit {
                code: status.code(),
                success: status.success(),
            };
            let _ = record_and_deliver(&inner, &session_id, kind, &delivery, &mut delivery_lagged);
            let session_status = if cancelling {
                SessionStatus::Cancelled
            } else if status.success() {
                SessionStatus::Completed
            } else {
                SessionStatus::Failed
            };
            let _ = update_session(
                &inner.storage,
                &session_id,
                session_status,
                status.code(),
                None,
            );
        }
        Err(error) => {
            let _ = update_session(
                &inner.storage,
                &session_id,
                SessionStatus::Failed,
                None,
                Some(&error.to_string()),
            );
        }
    }
    if let Ok(mut running) = inner.running.lock() {
        running.remove(&session_id);
    }
}

fn record_and_deliver(
    inner: &SupervisorInner,
    session_id: &str,
    kind: ProcessEventKind,
    delivery: &mpsc::Sender<ProcessEvent>,
    delivery_lagged: &mut bool,
) -> Result<(), ProcessError> {
    let event = append_event(&inner.storage, session_id, &kind)?;
    match delivery.try_send(event) {
        Ok(()) => *delivery_lagged = false,
        Err(mpsc::error::TrySendError::Full(_)) if !*delivery_lagged => {
            *delivery_lagged = true;
            append_event(
                &inner.storage,
                session_id,
                &ProcessEventKind::DeliveryLagged,
            )?;
        }
        Err(_) => {}
    }
    Ok(())
}

#[cfg(unix)]
fn terminate_process(pid: Option<u32>, force: bool, child: &mut Child) {
    if let Some(pid) = pid {
        let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
        // SAFETY: the spawned child is placed in a process group whose id is its positive PID.
        // A negative id targets only that group, never an arbitrary process selected by user input.
        unsafe {
            libc::kill(-(pid as i32), signal);
        }
    }
    if force {
        let _ = child.start_kill();
    }
}

#[cfg(not(unix))]
fn terminate_process(_pid: Option<u32>, _force: bool, child: &mut Child) {
    let _ = child.start_kill();
}

fn insert_session(
    storage: &Storage,
    session_id: &str,
    spec: &ProcessSpec,
    pid: Option<u32>,
    redactor: &Redactor,
) -> Result<(), ProcessError> {
    let timestamp = now()?;
    let command = json!({
        "program": spec.program.to_string_lossy(),
        "arguments": spec.arguments.iter().map(|value| redactor(&value.to_string_lossy())).collect::<Vec<_>>(),
        "cwd": spec.cwd.to_string_lossy(),
        "removedEnvironment": spec.removed_environment.iter().map(|name| name.to_string_lossy()).collect::<Vec<_>>(),
    });
    storage.with_connection(|connection| {
        connection.execute(
            "INSERT INTO agent_sessions
             (id, task_id, worktree_id, adapter_kind, status, command_json, pid, created_at, started_at)
             VALUES (?1, ?2, ?3, ?4, 'running', ?5, ?6, ?7, ?7)",
            params![
                session_id,
                spec.task_id,
                spec.worktree_id,
                spec.adapter_kind,
                command.to_string(),
                pid,
                timestamp
            ],
        )?;
        Ok(())
    })?;
    Ok(())
}

fn append_event(
    storage: &Storage,
    session_id: &str,
    kind: &ProcessEventKind,
) -> Result<ProcessEvent, ProcessError> {
    let occurred_at = now()?;
    let payload = serde_json::to_string(kind)?;
    let sequence =
        storage.with_transaction(|transaction| {
            let sequence: i64 = transaction.query_row(
                "SELECT COALESCE(MAX(sequence), 0) + 1 FROM agent_events WHERE session_id = ?1",
                [session_id],
                |row| row.get(0),
            )?;
            transaction.execute(
            "INSERT INTO agent_events (session_id, sequence, event_type, payload_json, occurred_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![session_id, sequence, kind.event_type(), payload, occurred_at],
        )?;
            Ok(sequence)
        })?;
    Ok(ProcessEvent {
        session_id: session_id.to_owned(),
        sequence,
        kind: kind.clone(),
        occurred_at,
    })
}

fn load_events(storage: &Storage, session_id: &str) -> Result<Vec<ProcessEvent>, ProcessError> {
    let raw: Vec<(i64, String, String)> = storage.with_connection(|connection| {
        let mut statement = connection.prepare(
            "SELECT sequence, payload_json, occurred_at FROM agent_events
             WHERE session_id = ?1 ORDER BY sequence",
        )?;
        statement
            .query_map([session_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect()
    })?;
    raw.into_iter()
        .map(|(sequence, payload, occurred_at)| {
            Ok(ProcessEvent {
                session_id: session_id.to_owned(),
                sequence,
                kind: serde_json::from_str(&payload)?,
                occurred_at,
            })
        })
        .collect()
}

fn load_session(storage: &Storage, session_id: &str) -> Result<SessionRecord, ProcessError> {
    let row = storage.with_connection(|connection| {
        connection
            .query_row(
                "SELECT id, status, pid, exit_code, error FROM agent_sessions WHERE id = ?1",
                [session_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<u32>>(2)?,
                        row.get::<_, Option<i32>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()
    })?;
    let Some((id, status, pid, exit_code, error)) = row else {
        return Err(ProcessError::SessionNotFound(session_id.to_owned()));
    };
    Ok(SessionRecord {
        id,
        status: SessionStatus::parse(&status)?,
        pid,
        exit_code,
        error,
    })
}

fn set_cancelling(storage: &Storage, session_id: &str) -> Result<(), ProcessError> {
    storage.with_connection(|connection| {
        connection.execute(
            "UPDATE agent_sessions SET status = 'cancelling' WHERE id = ?1 AND status = 'running'",
            [session_id],
        )?;
        Ok(())
    })?;
    Ok(())
}

fn update_session(
    storage: &Storage,
    session_id: &str,
    status: SessionStatus,
    exit_code: Option<i32>,
    error: Option<&str>,
) -> Result<(), ProcessError> {
    let timestamp = now()?;
    storage.with_connection(|connection| {
        connection.execute(
            "UPDATE agent_sessions SET status = ?2, exit_code = ?3, error = ?4, ended_at = ?5 WHERE id = ?1",
            params![session_id, status.as_str(), exit_code, error, timestamp],
        )?;
        Ok(())
    })?;
    Ok(())
}

fn now() -> Result<String, time::error::Format> {
    OffsetDateTime::now_utc().format(&Rfc3339)
}
