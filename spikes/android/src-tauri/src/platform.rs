//! The Android-only pieces, done in Kotlin: the Wi-Fi multicast lock
//! (`MulticastPlugin.kt`) and the foreground service (`ServicePlugin.kt`).

use tauri::plugin::{Builder, TauriPlugin};
use tauri::{AppHandle, Runtime};

#[cfg(target_os = "android")]
#[derive(serde::Serialize)]
struct OnArgs {
    on: bool,
}

#[cfg(target_os = "android")]
#[derive(serde::Deserialize)]
struct LockState {
    held: bool,
}

#[cfg(target_os = "android")]
#[derive(serde::Deserialize)]
struct ServiceState {
    running: bool,
}

#[cfg(target_os = "android")]
struct Multicast<R: Runtime>(tauri::plugin::PluginHandle<R>);

#[cfg(target_os = "android")]
struct Service<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// One Tauri plugin per Kotlin class: Android keys loaded plugins by the
/// Tauri name, so a second class under the same name replaces the first.
pub fn multicast<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("multicast")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                let handle = api.register_android_plugin(PACKAGE, "MulticastPlugin")?;
                app.manage(Multicast(handle));
            }
            #[cfg(not(target_os = "android"))]
            let _ = (app, api);
            Ok(())
        })
        .build()
}

pub fn service<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("service")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                let handle = api.register_android_plugin(PACKAGE, "ServicePlugin")?;
                app.manage(Service(handle));
            }
            #[cfg(not(target_os = "android"))]
            let _ = (app, api);
            Ok(())
        })
        .build()
}

#[cfg(target_os = "android")]
const PACKAGE: &str = "com.martcpp.takkiespike";

/// Takes or releases the lock and returns whether it's held now.
pub fn set_lock<R: Runtime>(app: &AppHandle<R>, on: bool) -> Result<bool, String> {
    #[cfg(target_os = "android")]
    {
        handle::<Multicast<R>>(app)?
            .0
            .run_mobile_plugin::<LockState>("setLock", OnArgs { on })
            .map(|state| state.held)
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, on);
        Err("the multicast lock only exists on Android".into())
    }
}

/// Starts or stops the foreground service and returns whether it runs now.
pub fn set_service<R: Runtime>(app: &AppHandle<R>, on: bool) -> Result<bool, String> {
    #[cfg(target_os = "android")]
    {
        handle::<Service<R>>(app)?
            .0
            .run_mobile_plugin::<ServiceState>("setService", OnArgs { on })
            .map(|state| state.running)
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, on);
        Err("the foreground service only exists on Android".into())
    }
}

#[cfg(target_os = "android")]
fn handle<T: Send + Sync + 'static>(
    app: &AppHandle<impl Runtime>,
) -> Result<tauri::State<'_, T>, String> {
    use tauri::Manager;
    app.try_state::<T>()
        .ok_or_else(|| "plugin isn't loaded".to_string())
}
