//! What only a phone needs, done in Kotlin under `gen/android`. On a
//! computer every call answers as if nothing were needed.

use serde::{Deserialize, Serialize};
use tauri::plugin::{Builder, TauriPlugin};
use tauri::{AppHandle, Runtime};

/// Whether the user has allowed something.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Grant {
    /// Allowed.
    Granted,
    /// Refused; only the system settings can change it now.
    Denied,
    /// Not asked yet.
    Prompt,
    /// Refused once; asking again is allowed and should say why.
    PromptWithRationale,
}

/// The runtime permissions the radio uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Access {
    /// To talk. The engine can't start without it.
    pub microphone: Grant,
    /// To show that the radio is on while the app is in the background.
    pub notifications: Grant,
}

impl Access {
    #[cfg(not(target_os = "android"))]
    const EVERYTHING: Self = Self {
        microphone: Grant::Granted,
        notifications: Grant::Granted,
    };
}

#[cfg(target_os = "android")]
struct Permissions<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// Loads the Kotlin side. Android keeps one plugin per name, so each Kotlin
/// class gets a plugin of its own.
pub fn permissions<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("permissions")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                let handle =
                    api.register_android_plugin("com.martcpp.takkie", "PermissionsPlugin")?;
                app.manage(Permissions(handle));
            }
            #[cfg(not(target_os = "android"))]
            let _ = (app, api);
            Ok(())
        })
        .build()
}

#[cfg(target_os = "android")]
fn kotlin<R: Runtime, T: serde::de::DeserializeOwned>(
    app: &AppHandle<R>,
    command: &str,
) -> Result<T, String> {
    use tauri::Manager;
    app.try_state::<Permissions<R>>()
        .ok_or_else(|| "the permissions plugin isn't loaded".to_owned())?
        .0
        .run_mobile_plugin(command, ())
        .map_err(|error| error.to_string())
}

/// What is allowed right now. Never call it on the main thread: Kotlin
/// answers there.
pub fn access<R: Runtime>(app: &AppHandle<R>) -> Result<Access, String> {
    #[cfg(target_os = "android")]
    {
        kotlin(app, "state")
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Ok(Access::EVERYTHING)
    }
}

/// Shows the system prompts for whatever is still missing, and returns what
/// is allowed afterwards.
pub fn ask<R: Runtime>(app: &AppHandle<R>) -> Result<Access, String> {
    #[cfg(target_os = "android")]
    {
        kotlin(app, "ask")
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Ok(Access::EVERYTHING)
    }
}

/// Opens the system's settings page for this app.
pub fn open_settings<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        kotlin::<R, Option<serde::de::IgnoredAny>>(app, "openSettings").map(drop)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Ok(())
    }
}

#[cfg(target_os = "android")]
struct Multicast<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// Loads the Kotlin side of the Wi-Fi multicast lock.
pub fn multicast<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("multicast")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                let handle =
                    api.register_android_plugin("com.martcpp.takkie", "MulticastPlugin")?;
                app.manage(Multicast(handle));
            }
            #[cfg(not(target_os = "android"))]
            let _ = (app, api);
            Ok(())
        })
        .build()
}

/// Takes or releases the Wi-Fi multicast lock, and returns whether it is
/// held now. Android drops incoming mDNS without it; elsewhere there is no
/// such lock and discovery always works, so this answers `true`.
pub fn set_multicast_lock<R: Runtime>(app: &AppHandle<R>, on: bool) -> Result<bool, String> {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;

        #[derive(Serialize)]
        struct Wanted {
            on: bool,
        }
        #[derive(Deserialize)]
        struct Lock {
            held: bool,
        }
        app.try_state::<Multicast<R>>()
            .ok_or_else(|| "the multicast plugin isn't loaded".to_owned())?
            .0
            .run_mobile_plugin::<Lock>("setLock", Wanted { on })
            .map(|lock| lock.held)
            .map_err(|error| error.to_string())
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, on);
        Ok(true)
    }
}

#[cfg(target_os = "android")]
struct Service<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// Loads the Kotlin side of the foreground service.
pub fn service<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("service")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                let handle = api.register_android_plugin("com.martcpp.takkie", "ServicePlugin")?;
                app.manage(Service(handle));
            }
            #[cfg(not(target_os = "android"))]
            let _ = (app, api);
            Ok(())
        })
        .build()
}

/// Starts or stops the foreground service that keeps the microphone and
/// discovery alive with the screen off. It can only start while the app is
/// on screen. A computer needs no such thing, so there this does nothing.
pub fn set_service<R: Runtime>(app: &AppHandle<R>, on: bool) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;

        #[derive(Serialize)]
        struct Wanted {
            on: bool,
        }
        app.try_state::<Service<R>>()
            .ok_or_else(|| "the service plugin isn't loaded".to_owned())?
            .0
            .run_mobile_plugin::<serde::de::IgnoredAny>("setService", Wanted { on })
            .map(drop)
            .map_err(|error| error.to_string())
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, on);
        Ok(())
    }
}

/// Blocks until the user taps Stop on the service's notification, which
/// may be hours. Only for a thread of its own.
#[cfg(target_os = "android")]
pub fn wait_for_stop<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    use tauri::Manager;
    app.try_state::<Service<R>>()
        .ok_or_else(|| "the service plugin isn't loaded".to_owned())?
        .0
        .run_mobile_plugin::<Option<serde::de::IgnoredAny>>("waitForStop", ())
        .map(drop)
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "android")]
struct Screen<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// Loads the Kotlin side of "keep the screen on".
pub fn screen<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("screen")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                let handle = api.register_android_plugin("com.martcpp.takkie", "ScreenPlugin")?;
                app.manage(Screen(handle));
            }
            #[cfg(not(target_os = "android"))]
            let _ = (app, api);
            Ok(())
        })
        .build()
}

/// Stops the screen sleeping while the app is in front, or lets it sleep
/// again. A computer is left alone.
pub fn keep_screen_on<R: Runtime>(app: &AppHandle<R>, on: bool) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;

        #[derive(Serialize)]
        struct Wanted {
            on: bool,
        }
        app.try_state::<Screen<R>>()
            .ok_or_else(|| "the screen plugin isn't loaded".to_owned())?
            .0
            .run_mobile_plugin::<serde::de::IgnoredAny>("setKeepOn", Wanted { on })
            .map(drop)
            .map_err(|error| error.to_string())
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, on);
        Ok(())
    }
}

#[cfg(target_os = "android")]
struct Network<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// Loads the Kotlin side of "are we on Wi-Fi".
pub fn network<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("network")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                let handle = api.register_android_plugin("com.martcpp.takkie", "NetworkPlugin")?;
                app.manage(Network(handle));
            }
            #[cfg(not(target_os = "android"))]
            let _ = (app, api);
            Ok(())
        })
        .build()
}

/// Whether this device is on a network other takkies can be on. On a phone
/// that means Wi-Fi or Ethernet: mobile data doesn't count. When it can't be
/// told, the answer is yes, so nobody is warned for nothing.
pub fn on_local_network<R: Runtime>(app: &AppHandle<R>) -> bool {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;

        #[derive(Deserialize)]
        struct State {
            local: bool,
        }
        app.try_state::<Network<R>>()
            .and_then(|network| network.0.run_mobile_plugin::<State>("state", ()).ok())
            .is_none_or(|state| state.local)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        !takkie_engine::net::address::local_networks().is_empty()
    }
}

/// Where a phone's sound comes out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Route {
    /// The loudspeaker.
    Speaker,
    /// A wired or USB headset.
    Wired,
    /// A Bluetooth headset or speaker.
    Bluetooth,
}

#[cfg(target_os = "android")]
struct Routing<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// Loads the Kotlin side of "where does sound come out".
pub fn route<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("route")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                let handle = api.register_android_plugin("com.martcpp.takkie", "RoutePlugin")?;
                app.manage(Routing(handle));
            }
            #[cfg(not(target_os = "android"))]
            let _ = (app, api);
            Ok(())
        })
        .build()
}

/// Where sound comes out right now, on a phone. `None` on a computer, where
/// the engine's device name already says it, and when Android won't tell.
pub fn current_route<R: Runtime>(app: &AppHandle<R>) -> Option<Route> {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;

        #[derive(Deserialize)]
        struct State {
            route: Route,
        }
        app.try_state::<Routing<R>>()?
            .0
            .run_mobile_plugin::<State>("state", ())
            .ok()
            .map(|state| state.route)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        None
    }
}

#[cfg(target_os = "android")]
struct Headset<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// Loads the Kotlin side of the headset button.
pub fn headset<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("headset")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                let handle = api.register_android_plugin("com.martcpp.takkie", "HeadsetPlugin")?;
                app.manage(Headset(handle));
            }
            #[cfg(not(target_os = "android"))]
            let _ = (app, api);
            Ok(())
        })
        .build()
}

/// Blocks until the headset button is pressed (`true`) or released
/// (`false`), which may be hours. Only for a thread of its own. A computer
/// has no such button for us, and gets an error at once.
pub fn wait_for_button<R: Runtime>(app: &AppHandle<R>) -> Result<bool, String> {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;

        #[derive(Deserialize)]
        struct Button {
            pressed: bool,
        }
        app.try_state::<Headset<R>>()
            .ok_or_else(|| "the headset plugin isn't loaded".to_owned())?
            .0
            .run_mobile_plugin::<Button>("waitForButton", ())
            .map(|button| button.pressed)
            .map_err(|error| error.to_string())
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Err("only phones pass headset buttons to apps".to_owned())
    }
}

/// Lets the headset button talk, or gives it back to whatever plays music.
/// A computer has no such button for us, so there this does nothing.
pub fn set_headset_button<R: Runtime>(app: &AppHandle<R>, on: bool) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;

        #[derive(Serialize)]
        struct Wanted {
            on: bool,
        }
        app.try_state::<Headset<R>>()
            .ok_or_else(|| "the headset plugin isn't loaded".to_owned())?
            .0
            .run_mobile_plugin::<serde::de::IgnoredAny>("setEnabled", Wanted { on })
            .map(drop)
            .map_err(|error| error.to_string())
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, on);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_use_the_words_kotlin_sends() {
        for (route, word) in [
            (Route::Speaker, "\"speaker\""),
            (Route::Wired, "\"wired\""),
            (Route::Bluetooth, "\"bluetooth\""),
        ] {
            assert_eq!(serde_json::to_string(&route).unwrap(), word);
            assert_eq!(serde_json::from_str::<Route>(word).unwrap(), route);
        }
    }

    #[test]
    fn what_kotlin_sends_is_understood() {
        let access: Access =
            serde_json::from_str(r#"{"microphone":"granted","notifications":"prompt"}"#).unwrap();
        assert_eq!(access.microphone, Grant::Granted);
        assert_eq!(access.notifications, Grant::Prompt);
        let access: Access = serde_json::from_str(
            r#"{"microphone":"prompt-with-rationale","notifications":"denied"}"#,
        )
        .unwrap();
        assert_eq!(access.microphone, Grant::PromptWithRationale);
        assert_eq!(access.notifications, Grant::Denied);
    }

    #[test]
    fn the_frontend_gets_the_same_words() {
        let access = Access {
            microphone: Grant::PromptWithRationale,
            notifications: Grant::Granted,
        };
        let json = serde_json::to_value(access).unwrap();
        assert_eq!(json["microphone"], "prompt-with-rationale");
        assert_eq!(json["notifications"], "granted");
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn a_computer_needs_no_asking() {
        assert_eq!(Access::EVERYTHING.microphone, Grant::Granted);
        assert_eq!(Access::EVERYTHING.notifications, Grant::Granted);
    }
}
