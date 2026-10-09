#[cfg(target_os = "android")]
mod aaudio;
mod audio;
mod discovery;
mod platform;

use std::sync::Mutex;

use audio::{Backend, Loopback, Stats};
use discovery::Discovery;
use tauri::{AppHandle, State};

#[derive(Default)]
struct Running(Mutex<Option<Loopback>>);

#[derive(Default)]
struct Discovering(Mutex<Option<Discovery>>);

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

// Async so it runs off the main thread, which the Kotlin side needs to answer.
#[tauri::command]
async fn set_lock(app: AppHandle, on: bool) -> Result<bool, String> {
    platform::set_lock(&app, on)
}

#[tauri::command]
async fn set_service(app: AppHandle, on: bool) -> Result<bool, String> {
    platform::set_service(&app, on)
}

#[tauri::command]
fn discover(state: State<'_, Discovering>, name: String) -> Result<String, String> {
    let mut slot = state.0.lock().map_err(|_| "state poisoned")?;
    if let Some(old) = slot.take() {
        old.stop();
    }
    let discovery = Discovery::start(&name)?;
    let me = discovery.me.clone();
    *slot = Some(discovery);
    Ok(me)
}

#[tauri::command]
fn undiscover(state: State<'_, Discovering>) -> Result<(), String> {
    let mut slot = state.0.lock().map_err(|_| "state poisoned")?;
    if let Some(discovery) = slot.take() {
        discovery.stop();
    }
    Ok(())
}

#[tauri::command]
fn peers(state: State<'_, Discovering>) -> Result<Vec<String>, String> {
    let slot = state.0.lock().map_err(|_| "state poisoned")?;
    Ok(slot.as_ref().map(Discovery::peers).unwrap_or_default())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(platform::multicast())
        .plugin(platform::service())
        .manage(Running::default())
        .manage(Discovering::default())
        .invoke_handler(tauri::generate_handler![
            audio_info,
            start,
            stop,
            ping,
            stats,
            set_lock,
            set_service,
            discover,
            undiscover,
            peers
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
