use xcap::Monitor;

pub fn get_main_monitor() -> Monitor {
    let monitors = Monitor::all().expect("Monitors not found");

    return monitors
        .into_iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .expect("No primary monitor found");
}
