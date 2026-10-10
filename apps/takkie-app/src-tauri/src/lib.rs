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
/// Carries the reason, once, when the engine stops while the app runs.
const ENGINE_STOPPED: &str = "engine-stopped";
const SNAPSHOT_EVERY: Duration = Duration::from_millis(100);

// Ends by itself once the engine is stopped, or if it never started.
fn publish_snapshots(app: AppHandle) {
    let publishing = thread::Builder::new()
        .name("takkie-app-live".into())
        .spawn(move || {
            let why = loop {
                match app.state::<Radio>().view() {
                    Ok(view) => {
                        if let Err(error) = app.emit(ENGINE_SNAPSHOT, view) {
                            tracing::debug!("snapshot not sent: {error}");
                        }
                        thread::sleep(SNAPSHOT_EVERY);
                    }
                    Err(why) => break why,
                }
            };
            if let Err(error) = app.emit(ENGINE_STOPPED, why) {
                tracing::debug!("stop not sent: {error}");
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
    // Before the engine, so its first mDNS answers aren't dropped.
    match mobile::set_multicast_lock(&app, true) {
        Ok(true) => {}
        Ok(false) => tracing::warn!("the Wi-Fi multicast lock wasn't taken; discovery may be slow"),
        Err(error) => tracing::warn!("no Wi-Fi multicast lock: {error}"),
    }
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
        if let Err(error) = mobile::set_service(&app, true) {
            tracing::warn!("no foreground service, so the radio stops with the screen: {error}");
        }
        stop_on_request(app.clone());
        publish_snapshots(app);
    }
    Ok(())
}

/// Turns the radio off until `start_radio` is called again.
fn turn_off(app: &AppHandle) {
    app.state::<Radio>().stop("You turned the radio off.");
    tracing::info!("radio turned off");
    if let Err(error) = mobile::set_service(app, false) {
        tracing::debug!("service not stopped: {error}");
    }
    if let Err(error) = mobile::set_multicast_lock(app, false) {
        tracing::debug!("multicast lock not released: {error}");
    }
}

// The notification's Stop action, which works with the app in the background.
#[cfg(target_os = "android")]
fn stop_on_request(app: AppHandle) {
    let waiting = thread::Builder::new()
        .name("takkie-app-stop".into())
        .spawn(move || match mobile::wait_for_stop(&app) {
            Ok(()) => turn_off(&app),
            Err(error) => tracing::warn!("the notification's Stop won't work: {error}"),
        });
    if let Err(error) = waiting {
        tracing::warn!("the notification's Stop won't work: {error}");
    }
}

#[cfg(not(target_os = "android"))]
fn stop_on_request(_app: AppHandle) {}

#[tauri::command(async)]
fn stop_radio(app: AppHandle) {
    turn_off(&app);
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
        .plugin(mobile::multicast())
        .plugin(mobile::service())
        .manage(Radio::new())
        .invoke_handler(tauri::generate_handler![
            start_radio,
            stop_radio,
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
            app.state::<Radio>().stop("the app is closing");
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
