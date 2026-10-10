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

#[cfg(test)]
mod tests {
    use super::*;

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
