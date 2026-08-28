use std::fs;
use std::sync::Arc;

use mars_core::Core;
use mars_core::files::{FileDocument, FileError, FilePage, FileService};
use mars_core::projects::{OpenRepositoryOutcome, ProjectError, ProjectService};
use mars_core::protocol::{CommandRequest, CommandResponse};
use mars_core::terminal::{TerminalError, TerminalService, TerminalSessionInfo};
use mars_security::TrustDecision;
use mars_storage::Storage;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

struct AppState {
    files: FileService,
    projects: ProjectService,
    terminals: TerminalService,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CommandFailure {
    code: &'static str,
    message: String,
}

impl From<ProjectError> for CommandFailure {
    fn from(error: ProjectError) -> Self {
        let code = match error {
            ProjectError::NotDirectory(_) => "repository.not_directory",
            ProjectError::NotGitRepository(_) => "repository.not_git",
            ProjectError::MalformedGitFile(_) => "repository.invalid_git_file",
            ProjectError::MissingName(_) => "repository.invalid_name",
            ProjectError::Filesystem { .. } => "repository.filesystem",
            ProjectError::Storage(_) => "repository.storage",
        };
        Self {
            code,
            message: error.to_string(),
        }
    }
}

impl From<FileError> for CommandFailure {
    fn from(error: FileError) -> Self {
        let code = match error {
            FileError::Storage(_) => "file.storage",
            FileError::InvalidRelativePath(_) => "file.invalid_path",
            FileError::OutsideRepository(_) => "file.outside_repository",
            FileError::IgnoredPath(_) => "file.ignored",
            FileError::NotFile(_) => "file.not_file",
            FileError::BinaryFile(_) => "file.binary",
            FileError::FileTooLarge { .. } => "file.too_large",
            FileError::WriteConflict { .. } => "file.write_conflict",
            FileError::Walk(_) => "file.walk",
            FileError::Filesystem { .. } => "file.filesystem",
        };
        Self {
            code,
            message: error.to_string(),
        }
    }
}

impl From<TerminalError> for CommandFailure {
    fn from(error: TerminalError) -> Self {
        let code = match error {
            TerminalError::Storage(_) => "terminal.storage",
            TerminalError::InvalidDimensions => "terminal.invalid_dimensions",
            TerminalError::SessionNotFound(_) => "terminal.not_found",
            TerminalError::Closed(_) => "terminal.closed",
            TerminalError::StatePoisoned(_) => "terminal.state",
            TerminalError::Pty(_) => "terminal.pty",
            TerminalError::Io(_) => "terminal.io",
        };
        Self {
            code,
            message: error.to_string(),
        }
    }
}

#[tauri::command]
fn core_info(request: CommandRequest) -> CommandResponse {
    Core::new().execute(request)
}

#[tauri::command]
fn open_repository(
    path: String,
    trust: TrustDecision,
    state: State<'_, AppState>,
) -> Result<OpenRepositoryOutcome, CommandFailure> {
    state
        .projects
        .open_repository(path, trust)
        .map_err(Into::into)
}

#[tauri::command]
fn list_files(
    repository_id: String,
    cursor: Option<String>,
    limit: usize,
    state: State<'_, AppState>,
) -> Result<FilePage, CommandFailure> {
    state
        .files
        .list_files(&repository_id, cursor.as_deref(), limit)
        .map_err(Into::into)
}

#[tauri::command]
fn read_file(
    repository_id: String,
    relative_path: String,
    state: State<'_, AppState>,
) -> Result<FileDocument, CommandFailure> {
    state
        .files
        .read_file(&repository_id, &relative_path)
        .map_err(Into::into)
}

#[tauri::command]
fn write_file(
    repository_id: String,
    relative_path: String,
    content: String,
    expected_version: String,
    state: State<'_, AppState>,
) -> Result<FileDocument, CommandFailure> {
    state
        .files
        .write_file(&repository_id, &relative_path, &content, &expected_version)
        .map_err(Into::into)
}

#[tauri::command]
fn spawn_terminal(
    repository_id: String,
    columns: u16,
    rows: u16,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<TerminalSessionInfo, CommandFailure> {
    let output_app = app.clone();
    state
        .terminals
        .spawn_terminal(
            &repository_id,
            columns,
            rows,
            Arc::new(move |output| {
                let _ = output_app.emit("terminal-output", output);
            }),
        )
        .map_err(Into::into)
}

#[tauri::command]
fn write_terminal(
    session_id: String,
    data: String,
    state: State<'_, AppState>,
) -> Result<(), CommandFailure> {
    state
        .terminals
        .write_terminal(&session_id, &data)
        .map_err(Into::into)
}

#[tauri::command]
fn resize_terminal(
    session_id: String,
    columns: u16,
    rows: u16,
    state: State<'_, AppState>,
) -> Result<(), CommandFailure> {
    state
        .terminals
        .resize_terminal(&session_id, columns, rows)
        .map_err(Into::into)
}

#[tauri::command]
fn cancel_terminal(session_id: String, state: State<'_, AppState>) -> Result<(), CommandFailure> {
    state
        .terminals
        .cancel_terminal(&session_id)
        .map_err(Into::into)
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            fs::create_dir_all(&data_dir)?;
            let storage = Arc::new(Storage::open(data_dir.join("mars.sqlite3"))?);
            app.manage(AppState {
                files: FileService::new(storage.clone()),
                projects: ProjectService::new(storage.clone()),
                terminals: TerminalService::new(storage),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            core_info,
            open_repository,
            list_files,
            read_file,
            write_file,
            spawn_terminal,
            write_terminal,
            resize_terminal,
            cancel_terminal
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Mars desktop");
}
