//! What the frontend can ask the engine to do.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use takkie_core::{ChannelId, Passphrase};
use takkie_engine::{Engine, EngineConfig, EngineEvent};
use tauri::{AppHandle, State};

use crate::mobile::{self, Access};
use crate::view::{DevicesView, Running, SnapshotView, peer_id};

/// The running engine, or why there isn't one.
pub struct Radio {
    engine: Mutex<Result<Engine, String>>,
    running: Arc<Mutex<Running>>,
}

impl Radio {
    /// A radio that is off until [`start`](Self::start).
    pub fn new() -> Self {
        Self {
            engine: Mutex::new(Err("it hasn't been started yet".to_owned())),
            running: Arc::default(),
        }
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
    pub fn stop(&self) {
        if let Ok(mut engine) = self.engine.lock() {
            *engine = Err("the app is closing".to_owned());
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
pub fn snapshot(radio: State<'_, Radio>) -> Result<SnapshotView, String> {
    radio.view()
}

#[tauri::command]
pub fn set_transmitting(radio: State<'_, Radio>, on: bool) -> Result<(), String> {
    radio.with(|engine| engine.set_transmitting(on))
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
        radio.stop();
        let asked = radio.with(|engine| engine.set_muted(true));
        assert_eq!(
            asked,
            Err("The engine isn't running: the app is closing".to_owned())
        );
    }
}
