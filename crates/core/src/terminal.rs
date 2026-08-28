use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use mars_storage::Storage;
use mars_storage::projects::ProjectRepository;
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

pub const MAX_SCROLLBACK_BYTES: usize = 1024 * 1024;

pub type TerminalOutputSink = Arc<dyn Fn(TerminalOutput) + Send + Sync + 'static>;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSessionInfo {
    pub session_id: String,
    pub cwd: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalOutput {
    pub session_id: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum TerminalError {
    #[error(transparent)]
    Storage(#[from] mars_storage::StorageError),
    #[error("terminal dimensions must be between 1 and 1000 cells")]
    InvalidDimensions,
    #[error("terminal session `{0}` does not exist")]
    SessionNotFound(String),
    #[error("terminal session `{0}` is closed")]
    Closed(String),
    #[error("{0} lock was poisoned")]
    StatePoisoned(&'static str),
    #[error("PTY operation failed: {0}")]
    Pty(String),
    #[error("terminal I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

struct TerminalSession {
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    output: Arc<Mutex<BoundedOutput>>,
    closed: Arc<AtomicBool>,
}

#[derive(Default)]
struct BoundedOutput {
    bytes: VecDeque<u8>,
}

impl BoundedOutput {
    fn append(&mut self, chunk: &[u8]) {
        self.bytes.extend(chunk);
        let overflow = self.bytes.len().saturating_sub(MAX_SCROLLBACK_BYTES);
        self.bytes.drain(..overflow);
    }

    fn text(&self) -> String {
        let bytes: Vec<_> = self.bytes.iter().copied().collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

pub struct TerminalService {
    projects: ProjectRepository,
    sessions: Mutex<HashMap<String, Arc<TerminalSession>>>,
}

impl TerminalService {
    #[must_use]
    pub fn new(storage: Arc<Storage>) -> Self {
        Self {
            projects: ProjectRepository::new(storage),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    pub fn spawn_terminal(
        &self,
        repository_id: &str,
        columns: u16,
        rows: u16,
        output_sink: TerminalOutputSink,
    ) -> Result<TerminalSessionInfo, TerminalError> {
        let size = terminal_size(columns, rows)?;
        let repository = self.projects.find_repository(repository_id)?;
        let pair = native_pty_system()
            .openpty(size)
            .map_err(|error| TerminalError::Pty(error.to_string()))?;
        let mut command = CommandBuilder::new_default_prog();
        command.cwd(&repository.root_path);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        let mut child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| TerminalError::Pty(error.to_string()))?;
        let killer = child.clone_killer();
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| TerminalError::Pty(error.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| TerminalError::Pty(error.to_string()))?;
        drop(pair.slave);

        let session_id = Uuid::now_v7().to_string();
        let output = Arc::new(Mutex::new(BoundedOutput::default()));
        let closed = Arc::new(AtomicBool::new(false));
        let session = Arc::new(TerminalSession {
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            killer: Mutex::new(killer),
            output: output.clone(),
            closed: closed.clone(),
        });
        self.sessions()?.insert(session_id.clone(), session);

        let reader_session_id = session_id.clone();
        let reader_closed = closed.clone();
        thread::spawn(move || {
            let mut buffer = [0_u8; 8 * 1024];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        if let Ok(mut scrollback) = output.lock() {
                            scrollback.append(&buffer[..read]);
                        }
                        output_sink(TerminalOutput {
                            session_id: reader_session_id.clone(),
                            data: buffer[..read].to_vec(),
                        });
                    }
                }
            }
            reader_closed.store(true, Ordering::Release);
        });

        let waiter_closed = closed;
        thread::spawn(move || {
            let _ = child.wait();
            waiter_closed.store(true, Ordering::Release);
        });

        Ok(TerminalSessionInfo {
            session_id,
            cwd: repository.root_path,
        })
    }

    pub fn write_terminal(&self, session_id: &str, data: &str) -> Result<(), TerminalError> {
        let session = self.session(session_id)?;
        ensure_open(session_id, &session)?;
        let mut writer = session
            .writer
            .lock()
            .map_err(|_| TerminalError::StatePoisoned("terminal writer"))?;
        writer.write_all(data.as_bytes())?;
        writer.flush()?;
        Ok(())
    }

    pub fn resize_terminal(
        &self,
        session_id: &str,
        columns: u16,
        rows: u16,
    ) -> Result<(), TerminalError> {
        let size = terminal_size(columns, rows)?;
        let session = self.session(session_id)?;
        ensure_open(session_id, &session)?;
        session
            .master
            .lock()
            .map_err(|_| TerminalError::StatePoisoned("terminal master"))?
            .resize(size)
            .map_err(|error| TerminalError::Pty(error.to_string()))
    }

    pub fn cancel_terminal(&self, session_id: &str) -> Result<(), TerminalError> {
        let session = self.session(session_id)?;
        if !session.closed.swap(true, Ordering::AcqRel) {
            session
                .killer
                .lock()
                .map_err(|_| TerminalError::StatePoisoned("terminal killer"))?
                .kill()?;
        }
        self.sessions()?.remove(session_id);
        Ok(())
    }

    pub fn terminal_output(&self, session_id: &str) -> Result<String, TerminalError> {
        let session = self.session(session_id)?;
        let output = session
            .output
            .lock()
            .map_err(|_| TerminalError::StatePoisoned("terminal output"))?
            .text();
        Ok(output)
    }

    fn session(&self, session_id: &str) -> Result<Arc<TerminalSession>, TerminalError> {
        self.sessions()?
            .get(session_id)
            .cloned()
            .ok_or_else(|| TerminalError::SessionNotFound(session_id.to_owned()))
    }

    fn sessions(
        &self,
    ) -> Result<MutexGuard<'_, HashMap<String, Arc<TerminalSession>>>, TerminalError> {
        self.sessions
            .lock()
            .map_err(|_| TerminalError::StatePoisoned("terminal sessions"))
    }
}

fn terminal_size(columns: u16, rows: u16) -> Result<PtySize, TerminalError> {
    if columns == 0 || rows == 0 || columns > 1_000 || rows > 1_000 {
        return Err(TerminalError::InvalidDimensions);
    }
    Ok(PtySize {
        rows,
        cols: columns,
        pixel_width: 0,
        pixel_height: 0,
    })
}

fn ensure_open(session_id: &str, session: &TerminalSession) -> Result<(), TerminalError> {
    if session.closed.load(Ordering::Acquire) {
        return Err(TerminalError::Closed(session_id.to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{BoundedOutput, MAX_SCROLLBACK_BYTES};

    #[test]
    fn scrollback_keeps_only_the_newest_bounded_bytes() {
        let mut output = BoundedOutput::default();
        output.append(&vec![b'a'; MAX_SCROLLBACK_BYTES]);
        output.append(b"tail");

        assert_eq!(output.bytes.len(), MAX_SCROLLBACK_BYTES);
        assert!(output.text().ends_with("tail"));
    }
}
