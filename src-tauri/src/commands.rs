use std::sync::Mutex;

use tauri::ipc::{Channel, InvokeResponseBody};

use crate::monitors::{self, MonitorInfo};
use crate::streaming::{self, Recording, ShareConfig};

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
    let rec = streaming::start_recording(config, on_chunk)?;
    let previous = state.lock().unwrap().replace(rec);
    if let Some(previous) = previous {
        previous.stop()?;
    }

    Ok(())
}

#[tauri::command]
pub fn stop_share(state: tauri::State<Mutex<Option<Recording>>>) -> Result<(), String> {
    if let Some(rec) = state.lock().unwrap().take() {
        rec.stop()?;
    };
    Ok(())
}
