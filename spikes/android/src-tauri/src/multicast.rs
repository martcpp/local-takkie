//! Holds Android's Wi-Fi multicast lock through `MulticastPlugin.kt`.

use tauri::plugin::{Builder, TauriPlugin};
use tauri::{AppHandle, Runtime};

#[cfg(target_os = "android")]
#[derive(serde::Serialize)]
struct LockArgs {
    on: bool,
}

#[cfg(target_os = "android")]
#[derive(serde::Deserialize)]
struct LockState {
    held: bool,
}

#[cfg(target_os = "android")]
struct Multicast<R: Runtime>(tauri::plugin::PluginHandle<R>);

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("multicast")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                let handle =
                    api.register_android_plugin("com.martcpp.takkiespike", "MulticastPlugin")?;
                app.manage(Multicast(handle));
            }
            #[cfg(not(target_os = "android"))]
            let _ = (app, api);
            Ok(())
        })
        .build()
}

/// Takes or releases the lock and returns whether it's held now.
pub fn set_lock<R: Runtime>(app: &AppHandle<R>, on: bool) -> Result<bool, String> {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;
        let plugin = app
            .try_state::<Multicast<R>>()
            .ok_or("multicast plugin isn't loaded")?;
        plugin
            .0
            .run_mobile_plugin::<LockState>("setLock", LockArgs { on })
            .map(|state| state.held)
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, on);
        Err("the multicast lock only exists on Android".into())
    }
}
