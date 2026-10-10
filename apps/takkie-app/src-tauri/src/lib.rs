//! The local-takkie desktop and Android app: a Tauri window around the engine.

mod commands;
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

fn computer_name() -> String {
    let name = gethostname::gethostname()
        .to_string_lossy()
        .trim()
        .to_owned();
    if name.is_empty() {
        "takkie".to_owned()
    } else {
        name
    }
}

/// Runs the app until its window closes.
///
/// # Errors
/// [`tauri::Error`] if the app can't start.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() -> tauri::Result<()> {
    // Fails only if a logger is already set, which is fine.
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .try_init();
    let app = tauri::Builder::default()
        .setup(|app| {
            let config = EngineConfig {
                display_name: computer_name(),
                ..EngineConfig::default()
            };
            let events = app.handle().clone();
            app.manage(Radio::start(config, move |event| {
                tracing::debug!(?event, "engine event");
                let Some(view) = EventView::at(&event, Instant::now()) else {
                    return;
                };
                if let Err(error) = events.emit(ENGINE_EVENT, view) {
                    tracing::debug!("event not sent: {error}");
                }
            }));
            publish_snapshots(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
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
