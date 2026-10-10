//! The local-takkie desktop and Android app: a Tauri window around the engine.

mod commands;
mod mobile;
mod view;

use std::thread;
use std::time::{Duration, Instant};

use takkie_engine::EngineConfig;
use tauri::{AppHandle, Emitter, Manager, RunEvent};
use tracing_subscriber::EnvFilter;

use commands::Radio;
use view::EventView;

/// Carries one [`EventView`] each time the engine reports something.
const ENGINE_EVENT: &str = "engine-event";
/// Carries the latest [`view::SnapshotView`], ten times a second.
const ENGINE_SNAPSHOT: &str = "engine-snapshot";
const SNAPSHOT_EVERY: Duration = Duration::from_millis(100);

// Ends by itself once the engine is stopped, or if it never started.
fn publish_snapshots(app: AppHandle) {
    let publishing = thread::Builder::new()
        .name("takkie-app-live".into())
        .spawn(move || {
            while let Ok(view) = app.state::<Radio>().view() {
                if let Err(error) = app.emit(ENGINE_SNAPSHOT, view) {
                    tracing::debug!("snapshot not sent: {error}");
                }
                thread::sleep(SNAPSHOT_EVERY);
            }
        });
    if let Err(error) = publishing {
        tracing::warn!("the screen won't update by itself: {error}");
    }
}

/// The name peers see until the user picks one: the computer's name, or
/// on a phone, where that is just `localhost`, what the maker calls it.
fn device_name() -> String {
    usable(&gethostname::gethostname().to_string_lossy())
        .or_else(phone_name)
        .unwrap_or_else(|| "takkie".to_owned())
}

fn usable(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty() && name != "localhost").then(|| name.to_owned())
}

#[cfg(target_os = "android")]
fn phone_name() -> Option<String> {
    // The market name ("REDMI 15C") is only set by some makers; the model
    // ("25078RA3EA") always is.
    ["ro.product.marketname", "ro.product.model"]
        .into_iter()
        .find_map(|property| {
            let output = std::process::Command::new("getprop")
                .arg(property)
                .output()
                .ok()?;
            usable(&String::from_utf8_lossy(&output.stdout))
        })
}

#[cfg(not(target_os = "android"))]
fn phone_name() -> Option<String> {
    None
}

/// Starts the engine, once the frontend knows the microphone is allowed.
/// Doing nothing when it already runs lets the frontend call it freely.
#[tauri::command(async)]
fn start_radio(app: AppHandle) -> Result<(), String> {
    let config = EngineConfig {
        display_name: device_name(),
        ..EngineConfig::default()
    };
    let events = app.clone();
    let started = app.state::<Radio>().start(config, move |event| {
        tracing::debug!(?event, "engine event");
        let Some(view) = EventView::at(&event, Instant::now()) else {
            return;
        };
        if let Err(error) = events.emit(ENGINE_EVENT, view) {
            tracing::debug!("event not sent: {error}");
        }
    })?;
    if started {
        publish_snapshots(app);
    }
    Ok(())
}

/// Runs the app until its window closes.
///
/// # Errors
/// [`tauri::Error`] if the app can't start.
pub fn run() -> tauri::Result<()> {
    // Fails only if a logger is already set, which is fine.
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        // logcat shows colour codes as text.
        .with_ansi(!cfg!(target_os = "android"))
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .try_init();
    let app = tauri::Builder::default()
        .plugin(mobile::permissions())
        .manage(Radio::new())
        .invoke_handler(tauri::generate_handler![
            start_radio,
            commands::permissions,
            commands::request_permissions,
            commands::open_app_settings,
            commands::snapshot,
            commands::set_transmitting,
            commands::set_channel,
            commands::set_muted,
            commands::set_volume,
            commands::set_peer_muted,
            commands::set_beeps,
            commands::list_devices,
        ])
        .build(tauri::generate_context!())?;
    app.run(|app, event| {
        if let RunEvent::Exit = event {
            app.state::<Radio>().stop();
        }
    });
    Ok(())
}

// Android loads the library and calls this; there is no `main` to report to.
#[cfg(mobile)]
#[tauri::mobile_entry_point]
fn start() {
    if let Err(error) = run() {
        tracing::error!("the app couldn't start: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hostname_is_used_unless_it_says_nothing() {
        assert_eq!(
            usable(
                "  DESKTOP-1
"
            ),
            Some("DESKTOP-1".to_owned())
        );
        assert_eq!(usable("REDMI 15C"), Some("REDMI 15C".to_owned()));
        assert_eq!(usable("localhost"), None);
        assert_eq!(
            usable(
                " 
"
            ),
            None
        );
        assert_eq!(usable(""), None);
    }

    #[test]
    fn there_is_always_a_name() {
        assert!(!device_name().is_empty());
    }
}
