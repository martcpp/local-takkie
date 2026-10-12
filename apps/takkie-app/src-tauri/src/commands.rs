//! What the frontend can ask the engine to do.

use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use takkie_core::{ChannelId, Passphrase};
use takkie_engine::{Engine, EngineConfig, EngineEvent, EngineStats, Strength};
use tauri::{AppHandle, State};

use crate::mobile::{self, Access, Route};
use crate::view::{DevicesView, Running, SnapshotView, peer_id};

/// The running engine, or why there isn't one.
pub struct Radio {
    engine: Mutex<Result<Engine, String>>,
    running: Arc<Mutex<Running>>,
    // Two things can ask to talk. The radio transmits while either does.
    screen_talks: AtomicBool,
    headset_talks: AtomicBool,
}

impl Radio {
    /// A radio that is off until [`start`](Self::start).
    pub fn new() -> Self {
        Self {
            engine: Mutex::new(Err("it hasn't been started yet".to_owned())),
            running: Arc::default(),
            screen_talks: AtomicBool::new(false),
            headset_talks: AtomicBool::new(false),
        }
    }

    /// The on-screen button, or the keyboard, was pressed or let go.
    pub fn talk_from_screen(&self, on: bool) -> Result<(), String> {
        self.screen_talks.store(on, Relaxed);
        self.talk()
    }

    /// The headset button now means talk, or no longer does.
    pub fn talk_from_headset(&self, on: bool) -> Result<(), String> {
        self.headset_talks.store(on, Relaxed);
        self.talk()
    }

    /// Whether the headset button is what keeps us talking.
    pub fn headset_talks(&self) -> bool {
        self.headset_talks.load(Relaxed)
    }

    /// How loud the microphone is right now, if the engine runs.
    pub fn mic_level(&self) -> Option<f32> {
        self.with(|engine| engine.snapshot().mic.rms).ok()
    }

    fn talk(&self) -> Result<(), String> {
        let on = self.screen_talks.load(Relaxed) || self.headset_talks.load(Relaxed);
        self.with(|engine| engine.set_transmitting(on))
    }

    /// Starts the engine and hands each of its events to `on_event`.
    /// Returns `false` if it was running already. A failure is kept, so the
    /// window can show it, and starting can be tried again.
    pub fn start(
        &self,
        config: EngineConfig,
        on_event: impl Fn(EngineEvent) + Send + 'static,
    ) -> Result<bool, String> {
        let mut engine = self
            .engine
            .lock()
            .map_err(|_| "the engine stopped unexpectedly".to_owned())?;
        if engine.is_ok() {
            return Ok(false);
        }
        let noting = Arc::clone(&self.running);
        let started = match Engine::start(config) {
            Ok((engine, events)) => {
                let forwarding = std::thread::Builder::new()
                    .name("takkie-app-events".into())
                    .spawn(move || {
                        for event in events {
                            if let Ok(mut running) = noting.lock() {
                                running.note(&event);
                            }
                            on_event(event);
                        }
                    });
                if let Err(error) = forwarding {
                    tracing::warn!("engine events won't be read: {error}");
                }
                Ok(engine)
            }
            Err(error) => {
                tracing::error!("the engine didn't start: {error}");
                Err(error.to_string())
            }
        };
        let outcome = started.as_ref().map(|_| true).map_err(Clone::clone);
        *engine = started;
        outcome
    }

    /// Stops the engine: threads joined, Bye sent, mDNS unregistered.
    /// `why` is what the window shows until it is started again.
    pub fn stop(&self, why: &str) {
        self.screen_talks.store(false, Relaxed);
        self.headset_talks.store(false, Relaxed);
        if let Ok(mut engine) = self.engine.lock() {
            *engine = Err(why.to_owned());
        }
    }

    /// Reopens the microphone and speaker where the system now sends sound.
    pub fn reopen_audio(&self) {
        let _ = self.with(Engine::reopen_audio);
    }

    /// The engine's running totals, for the log.
    pub fn stats(&self) -> Option<EngineStats> {
        self.with(|engine| engine.snapshot().stats).ok()
    }

    /// Whether the engine is running.
    pub fn is_running(&self) -> bool {
        self.engine.lock().is_ok_and(|engine| engine.is_ok())
    }

    /// Records whether this device is on a network other takkies can be on.
    pub fn note_network(&self, on_network: bool) {
        if let Ok(mut running) = self.running.lock() {
            running.note_network(on_network);
        }
    }

    /// Records where a phone's sound comes out.
    pub fn note_route(&self, route: Option<Route>) {
        if let Ok(mut running) = self.running.lock() {
            running.note_route(route);
        }
    }

    /// What the main screen draws right now.
    pub fn view(&self) -> Result<SnapshotView, String> {
        let running = self
            .running
            .lock()
            .map(|running| running.clone())
            .unwrap_or_default();
        self.with(|engine| SnapshotView::at(&engine.snapshot(), &running, Instant::now()))
    }

    fn with<T>(&self, action: impl FnOnce(&Engine) -> T) -> Result<T, String> {
        let engine: MutexGuard<'_, _> = self
            .engine
            .lock()
            .map_err(|_| "the engine stopped unexpectedly".to_owned())?;
        match &*engine {
            Ok(engine) => Ok(action(engine)),
            Err(why) => Err(format!("The engine isn't running: {why}")),
        }
    }
}

/// A channel and optional passphrase, as the frontend sends them.
fn join(
    channel: u8,
    passphrase: Option<String>,
) -> Result<(ChannelId, Option<Passphrase>), String> {
    let channel = ChannelId::try_from(channel).map_err(|error| error.to_string())?;
    // An empty passphrase means an open channel.
    let passphrase = passphrase.and_then(|secret| Passphrase::new(secret).ok());
    Ok((channel, passphrase))
}

// Async, like every command that reaches Kotlin: it answers on the main
// thread, which a plain command would be blocking.
#[tauri::command]
pub async fn permissions(app: AppHandle) -> Result<Access, String> {
    mobile::access(&app)
}

#[tauri::command]
pub async fn request_permissions(app: AppHandle) -> Result<Access, String> {
    mobile::ask(&app)
}

#[tauri::command]
pub async fn open_app_settings(app: AppHandle) -> Result<(), String> {
    mobile::open_settings(&app)
}

#[tauri::command]
pub async fn set_headset_button(
    app: AppHandle,
    radio: State<'_, Radio>,
    on: bool,
) -> Result<(), String> {
    if !on {
        // Don't leave it talking with no button left to stop it.
        let _ = radio.talk_from_headset(false);
    }
    mobile::set_headset_button(&app, on)
}

#[tauri::command]
pub async fn set_keep_screen_on(app: AppHandle, on: bool) -> Result<(), String> {
    mobile::keep_screen_on(&app, on)
}

#[tauri::command]
pub fn snapshot(radio: State<'_, Radio>) -> Result<SnapshotView, String> {
    radio.view()
}

#[tauri::command]
pub fn set_transmitting(radio: State<'_, Radio>, on: bool) -> Result<(), String> {
    radio.talk_from_screen(on)
}

#[tauri::command]
pub fn set_channel(
    radio: State<'_, Radio>,
    channel: u8,
    passphrase: Option<String>,
) -> Result<(), String> {
    let (channel, passphrase) = join(channel, passphrase)?;
    radio.with(|engine| engine.set_channel(channel, passphrase))
}

#[tauri::command]
pub fn set_muted(radio: State<'_, Radio>, muted: bool) -> Result<(), String> {
    radio.with(|engine| engine.set_muted(muted))
}

#[tauri::command]
pub fn set_volume(radio: State<'_, Radio>, volume: f32) -> Result<(), String> {
    radio.with(|engine| engine.set_volume(volume))
}

#[tauri::command]
pub fn set_peer_muted(radio: State<'_, Radio>, peer: &str, muted: bool) -> Result<(), String> {
    let peer = peer_id(peer).ok_or_else(|| format!("{peer:?} isn't a peer id"))?;
    radio.with(|engine| engine.set_peer_muted(peer, muted))
}

#[tauri::command]
pub fn set_beeps(radio: State<'_, Radio>, on: bool) -> Result<(), String> {
    radio.with(|engine| engine.set_beeps(on))
}

fn strength(name: &str) -> Result<Strength, String> {
    Strength::from_name(name).ok_or_else(|| format!("{name:?} isn't off, normal or strong"))
}

#[tauri::command]
pub fn set_noise_suppression(radio: State<'_, Radio>, level: &str) -> Result<(), String> {
    let level = strength(level)?;
    radio.with(|engine| engine.set_noise_suppression(level))
}

#[tauri::command]
pub fn set_auto_level(radio: State<'_, Radio>, on: bool) -> Result<(), String> {
    radio.with(|engine| engine.set_auto_level(on))
}

#[tauri::command]
pub fn set_clarity(radio: State<'_, Radio>, level: &str) -> Result<(), String> {
    let level = strength(level)?;
    radio.with(|engine| engine.set_clarity(level))
}

#[tauri::command(async)]
pub fn list_devices() -> Result<DevicesView, String> {
    let devices = Engine::list_devices().map_err(|error| error.to_string())?;
    Ok(DevicesView::from(&devices))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_channel_and_passphrase_from_the_frontend_are_checked() {
        let (channel, passphrase) = join(7, Some("correct horse".into())).unwrap();
        assert_eq!(channel.get(), 7);
        assert_eq!(passphrase.unwrap().expose_secret(), "correct horse");

        let (_, open) = join(1, None).unwrap();
        assert!(open.is_none());
        let (_, empty) = join(10, Some(String::new())).unwrap();
        assert!(empty.is_none());

        assert!(join(0, None).is_err());
        assert!(join(11, None).is_err());
    }

    #[test]
    fn a_stopped_radio_says_why_instead_of_acting() {
        let radio = Radio::new();
        assert_eq!(
            radio.view().err(),
            Some("The engine isn't running: it hasn't been started yet".to_owned())
        );
        if let Ok(mut engine) = radio.engine.lock() {
            *engine = Err("the port is taken".to_owned());
        }
        let asked = radio.with(|engine| engine.set_muted(true));
        assert_eq!(
            asked,
            Err("The engine isn't running: the port is taken".to_owned())
        );
        assert_eq!(radio.view().err(), asked.err());
        assert!(!radio.is_running());
        assert!(radio.talk_from_headset(true).is_err());
        assert!(radio.headset_talks.load(Relaxed));
        radio.stop("the app is closing");
        assert!(
            !radio.headset_talks.load(Relaxed),
            "stopping lets go of the button"
        );
        let asked = radio.with(|engine| engine.set_muted(true));
        assert_eq!(
            asked,
            Err("The engine isn't running: the app is closing".to_owned())
        );
    }
}
