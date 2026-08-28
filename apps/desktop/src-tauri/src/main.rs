use mars_core::Core;
use mars_core::protocol::{CommandRequest, CommandResponse};

#[tauri::command]
fn core_info(request: CommandRequest) -> CommandResponse {
    Core::new().execute(request)
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![core_info])
        .run(tauri::generate_context!())
        .expect("failed to run Mars desktop");
}
