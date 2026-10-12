//! The local-takkie desktop and Android app: a Tauri window around the engine.

mod commands;
mod headset;
mod mobile;
mod view;

use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
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
/// How often to look whether the engine still runs while nobody is watching.
const HIDDEN_EVERY: Duration = Duration::from_millis(500);

/// How often to look at the network and at where sound comes out.
const SURROUNDINGS_EVERY: Duration = Duration::from_secs(3);

/// Whether the window is on screen. Snapshots are only worth sending then.
struct OnScreen(AtomicBool);

// Ends by itself once the engine is stopped, or if it never started.
fn publish_snapshots(app: AppHandle) {
    let publishing = thread::Builder::new()
        .name("takkie-app-live".into())
        .spawn(move || {
            let why = loop {
                match app.state::<Radio>().view() {
                    Ok(view) => {
                        if !app.state::<OnScreen>().0.load(Relaxed) {
                            thread::sleep(HIDDEN_EVERY);
                        } else {
                            if let Err(error) = app.emit(ENGINE_SNAPSHOT, view) {
                                tracing::debug!("snapshot not sent: {error}");
                            }
                            thread::sleep(SNAPSHOT_EVERY);
                        }
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
        follow_headset(app.clone());
        watch_surroundings(app.clone());
        publish_snapshots(app);
    }
    Ok(())
}

// Follows what the engine can't see: the network we are on, and on a phone
// where sound comes out. Ends by itself once the engine is stopped.
fn watch_surroundings(app: AppHandle) {
    let watching = thread::Builder::new()
        .name("takkie-app-surroundings".into())
        .spawn(move || {
            let mut logged = None;
            let mut routed = None;
            while app.state::<Radio>().is_running() {
                let on_network = mobile::on_local_network(&app);
                let route = mobile::current_route(&app);
                let radio = app.state::<Radio>();
                radio.note_network(on_network);
                radio.note_route(route);
                // Android leaves open streams on the device they started on.
                if routed.is_some() && route != routed {
                    tracing::info!(?route, "sound moved; reopening audio");
                    radio.reopen_audio();
                }
                routed = route;
                // Audio trouble is only heard, never reported, so leave a trace.
                let stats = radio.stats();
                if stats != logged
                    && let Some(now) = stats
                {
                    tracing::info!(
                        underruns = now.speaker_underruns,
                        concealed = now.frames_concealed,
                        silenced = now.frames_silenced,
                        recovered = now.frames_recovered,
                        mic_dropped = now.mic_dropped,
                        block_ms = now.speaker_block_ms,
                        received = now.packets_received,
                        sent = now.packets_sent,
                        "audio totals"
                    );
                    logged = stats;
                }
                thread::sleep(SURROUNDINGS_EVERY);
            }
        });
    if let Err(error) = watching {
        tracing::warn!("a lost network or a new headset won't be shown: {error}");
    }
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

/// Set once the thread that follows the headset button is running.
struct HeadsetFollowed(AtomicBool);

/// How often the microphone level is looked at while a click keeps us talking.
const LISTEN_EVERY: Duration = Duration::from_millis(100);

// Someone who forgot the second click shouldn't leave their microphone open
// to the whole channel. Ends as soon as the talking does.
fn stop_when_quiet(app: AppHandle) {
    let listening = thread::Builder::new()
        .name("takkie-app-quiet".into())
        .spawn(move || {
            let mut watch = headset::SpeechWatch::new(Instant::now());
            loop {
                thread::sleep(LISTEN_EVERY);
                let radio = app.state::<Radio>();
                if !radio.headset_talks() {
                    return;
                }
                let Some(level) = radio.mic_level() else {
                    return;
                };
                let now = Instant::now();
                watch.hear(level, now);
                if watch.gone_quiet(now) {
                    tracing::info!("stopped talking: no speech for a while");
                    let _ = radio.talk_from_headset(false);
                    let stopped = EventView::Warning {
                        text: format!(
                            "Stopped talking: no speech for {} seconds",
                            headset::QUIET_LIMIT.as_secs()
                        ),
                    };
                    if let Err(error) = app.emit(ENGINE_EVENT, stopped) {
                        tracing::debug!("event not sent: {error}");
                    }
                    return;
                }
            }
        });
    if let Err(error) = listening {
        tracing::warn!("a forgotten microphone won't close by itself: {error}");
    }
}

// One thread for the life of the app: a second one would take every other
// press away from the first. On a computer it ends at once.
fn follow_headset(app: AppHandle) {
    if app.state::<HeadsetFollowed>().0.swap(true, Relaxed) {
        return;
    }
    let following = thread::Builder::new()
        .name("takkie-app-headset".into())
        .spawn(move || {
            loop {
                match mobile::wait_for_button(&app) {
                    // A click starts talking and the next click stops it.
                    // Holding can't work on a wired earpiece: its button
                    // shorts out its own microphone while it is down.
                    Ok(true) => {
                        let radio = app.state::<Radio>();
                        let talking = !radio.headset_talks();
                        tracing::info!(talking, "headset button");
                        if radio.talk_from_headset(talking).is_ok() && talking {
                            stop_when_quiet(app.clone());
                        }
                    }
                    Ok(false) => {}
                    Err(error) => {
                        tracing::debug!("not following a headset button: {error}");
                        return;
                    }
                }
            }
        });
    if let Err(error) = following {
        tracing::warn!("the headset button won't work: {error}");
    }
}

#[tauri::command(async)]
fn stop_radio(app: AppHandle) {
    turn_off(&app);
}

/// The frontend says when the window is shown or hidden, so a hidden one
/// isn't sent ten snapshots a second.
#[tauri::command]
fn set_on_screen(app: AppHandle, on: bool) {
    app.state::<OnScreen>().0.store(on, Relaxed);
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
        .plugin(mobile::screen())
        .plugin(mobile::network())
        .plugin(mobile::route())
        .plugin(mobile::headset())
        .manage(Radio::new())
        .manage(OnScreen(AtomicBool::new(true)))
        .manage(HeadsetFollowed(AtomicBool::new(false)))
        .invoke_handler(tauri::generate_handler![
            start_radio,
            stop_radio,
            set_on_screen,
            commands::permissions,
            commands::request_permissions,
            commands::open_app_settings,
            commands::set_keep_screen_on,
            commands::set_headset_button,
            commands::snapshot,
            commands::set_transmitting,
            commands::set_channel,
            commands::set_muted,
            commands::set_volume,
            commands::set_peer_muted,
            commands::set_beeps,
            commands::set_noise_suppression,
            commands::set_auto_level,
            commands::set_clarity,
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
