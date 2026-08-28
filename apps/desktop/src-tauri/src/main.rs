use std::fs;
use std::sync::Arc;

use mars_core::Core;
use mars_core::projects::{OpenRepositoryOutcome, ProjectError, ProjectService};
use mars_core::protocol::{CommandRequest, CommandResponse};
use mars_security::TrustDecision;
use mars_storage::Storage;
use serde::Serialize;
use tauri::{Manager, State};

struct AppState {
    projects: ProjectService,
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

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            fs::create_dir_all(&data_dir)?;
            let storage = Arc::new(Storage::open(data_dir.join("mars.sqlite3"))?);
            app.manage(AppState {
                projects: ProjectService::new(storage),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![core_info, open_repository])
        .run(tauri::generate_context!())
        .expect("failed to run Mars desktop");
}
