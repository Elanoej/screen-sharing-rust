use xcap::Monitor;

#[derive(serde::Serialize)]
pub struct MonitorInfo {
    pub id: String,
    pub label: String,
}

pub fn list_monitors() -> Result<Vec<MonitorInfo>, String> {
    let monitors = Monitor::all().map_err(|e| e.to_string())?;

    monitors
        .into_iter()
        .map(|monitor| {
            let id = monitor.id().map_err(|e| e.to_string())?;
            let name = monitor.name().map_err(|e| e.to_string())?;
            let width = monitor.width().map_err(|e| e.to_string())?;
            let height = monitor.height().map_err(|e| e.to_string())?;
            let primary = monitor.is_primary().map_err(|e| e.to_string())?;

            Ok(MonitorInfo {
                id: id.to_string(),
                label: format!(
                    "{name} · {width}×{height}{}",
                    if primary { " (principal)" } else { "" }
                ),
            })
        })
        .collect()
}

pub fn get_main_monitor() -> Monitor {
    let monitors = Monitor::all().expect("Monitors not found");

    return monitors
        .into_iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .expect("No primary monitor found");
}
