//! The local-takkie desktop and Android app: a Tauri window around the engine.

use serde::Serialize;
use takkie_engine::{DeviceList, Engine};

/// How many microphones and speakers the engine found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
struct DeviceCount {
    inputs: usize,
    outputs: usize,
}

impl From<&DeviceList> for DeviceCount {
    fn from(devices: &DeviceList) -> Self {
        Self {
            inputs: devices.inputs.len(),
            outputs: devices.outputs.len(),
        }
    }
}

#[tauri::command(async)]
fn device_count() -> Result<DeviceCount, String> {
    let devices = Engine::list_devices().map_err(|error| error.to_string())?;
    Ok(DeviceCount::from(&devices))
}

/// Runs the app until its window closes.
///
/// # Errors
/// [`tauri::Error`] if the app can't start.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() -> tauri::Result<()> {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![device_count])
        .run(tauri::generate_context!())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_device_list_counts_as_none() {
        assert_eq!(
            DeviceCount::from(&DeviceList::default()),
            DeviceCount {
                inputs: 0,
                outputs: 0
            }
        );
    }
}
