use crate::monitors::{self, MonitorInfo};

#[tauri::command]
pub fn available_monitors() -> Result<Vec<MonitorInfo>, String> {
    monitors::list_monitors()
}
