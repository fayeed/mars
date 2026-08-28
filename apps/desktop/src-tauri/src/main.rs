use mars_core::{Core, ProtocolVersion};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CoreInfo {
    protocol_version: ProtocolVersion,
}

#[tauri::command]
fn core_info() -> CoreInfo {
    CoreInfo {
        protocol_version: Core::new().protocol_version(),
    }
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![core_info])
        .run(tauri::generate_context!())
        .expect("failed to run Mars desktop");
}
