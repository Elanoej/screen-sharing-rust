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

pub fn get_monitor_by_id(id: u32) -> Result<Monitor, String> {
    Monitor::all()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|m| m.id().map(|mid| mid == id).unwrap_or(false))
        .ok_or_else(|| format!("Monitor com id {id} não encontrado"))
}
