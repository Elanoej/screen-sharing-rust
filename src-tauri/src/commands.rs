use std::sync::Mutex;

use tauri::ipc::{Channel, InvokeResponseBody};

use crate::monitors::{self, MonitorInfo};
use crate::screen_record::{self, Recording, ShareConfig};

#[tauri::command]
pub fn available_monitors() -> Result<Vec<MonitorInfo>, String> {
    monitors::list_monitors()
}

#[tauri::command]
pub fn start_share(
    config: ShareConfig,
    on_chunk: Channel<InvokeResponseBody>,
    state: tauri::State<Mutex<Option<Recording>>>,
) -> Result<(), String> {
    let rec = screen_record::start_recording(config, on_chunk)?;
    *state.lock().unwrap() = Some(rec);
    Ok(())
}

#[tauri::command]
pub fn stop_share(state: tauri::State<Mutex<Option<Recording>>>) -> Result<(), String> {
    if let Some(rec) = state.lock().unwrap().take() {
        rec.stop()?;
    };
    Ok(())
}
