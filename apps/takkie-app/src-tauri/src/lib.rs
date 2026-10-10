//! The local-takkie desktop and Android app: a Tauri window around the engine.

mod commands;
mod view;

use takkie_engine::EngineConfig;
use tauri::{Manager, RunEvent};
use tracing_subscriber::EnvFilter;

use commands::Radio;

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
            app.manage(Radio::start(config));
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
