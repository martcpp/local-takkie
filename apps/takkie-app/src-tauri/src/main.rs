//! The desktop binary. The app itself is in the library, which Android
//! loads directly.

// Without this, a release build on Windows opens a console window too.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;

fn main() -> ExitCode {
    match takkie_app_lib::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("takkie-app: {error}");
            ExitCode::FAILURE
        }
    }
}
