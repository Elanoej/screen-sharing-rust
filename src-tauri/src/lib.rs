mod commands;
mod conversions;
mod monitors;
mod streaming;
mod encoder;

use std::sync::Mutex;

use commands::{available_monitors, start_share, stop_share};
use streaming::Recording;

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(Mutex::new(None::<Recording>))
        .invoke_handler(tauri::generate_handler![
            greet,
            available_monitors,
            start_share,
            stop_share
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
