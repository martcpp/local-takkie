#[cfg(target_os = "android")]
mod aaudio;
mod audio;

use std::sync::Mutex;

use audio::{Backend, Loopback, Stats};
use tauri::State;

#[derive(Default)]
struct Running(Mutex<Option<Loopback>>);

#[tauri::command]
fn audio_info() -> String {
    audio::describe()
}

#[tauri::command]
fn start(running: State<'_, Running>, backend: Backend) -> Result<String, String> {
    let mut slot = running.0.lock().map_err(|_| "state poisoned")?;
    if let Some(old) = slot.take() {
        old.stop();
    }
    let (loopback, report) = Loopback::start(backend)?;
    *slot = Some(loopback);
    Ok(report)
}

#[tauri::command]
fn stop(running: State<'_, Running>) -> Result<(), String> {
    let mut slot = running.0.lock().map_err(|_| "state poisoned")?;
    if let Some(loopback) = slot.take() {
        loopback.stop();
    }
    Ok(())
}

#[tauri::command]
fn ping(running: State<'_, Running>) -> Result<(), String> {
    let slot = running.0.lock().map_err(|_| "state poisoned")?;
    slot.as_ref().ok_or("start the loopback first")?.ping();
    Ok(())
}

#[tauri::command]
fn stats(running: State<'_, Running>) -> Result<Option<Stats>, String> {
    let slot = running.0.lock().map_err(|_| "state poisoned")?;
    Ok(slot.as_ref().map(Loopback::stats))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Running::default())
        .invoke_handler(tauri::generate_handler![
            audio_info, start, stop, ping, stats
        ])
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while building tauri application");
}
