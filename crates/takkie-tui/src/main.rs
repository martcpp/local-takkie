//! Terminal app for local-takkie.

use std::env;
use std::io::{self, Write};
use std::process::ExitCode;

use clap::Parser;
use takkie_engine::net::address::local_networks;
use takkie_engine::{DeviceInfo, Engine, EngineConfig};

use cli::Cli;

mod cli;
mod logging;
mod settings;
mod ui;

fn main() -> ExitCode {
    let cli = Cli::parse();
    if cli.list_devices {
        return list_devices();
    }

    let filter = match logging::filter(
        cli.log_level.as_deref(),
        env::var("RUST_LOG").ok().as_deref(),
    ) {
        Ok(filter) => filter,
        Err(err) => {
            eprintln!("takkie: bad --log-level: {err}");
            return ExitCode::FAILURE;
        }
    };
    let logs = match logging::init(filter) {
        Ok(logs) => Some(logs),
        Err(err) => {
            eprintln!("takkie: logging is off: {err:#}");
            None
        }
    };

    let settings_path = cli.config.clone().or_else(settings::path);
    let loaded = settings_path.as_deref().map(settings::load);
    let (saved, settings_note) = match loaded {
        Some(settings::Loaded::Missing(saved) | settings::Loaded::Read(saved)) => {
            (Some(saved), None)
        }
        Some(settings::Loaded::Broken(why)) => {
            tracing::warn!("settings file ignored ({why}), using defaults");
            (None, None)
        }
        None => (
            None,
            Some("⚠️ No config directory, settings won't be saved".to_owned()),
        ),
    };
    let mut current = cli.apply(saved.clone().unwrap_or_default());
    let name = current.name.clone().unwrap_or_else(computer_name);

    let (engine, events) = match Engine::start(EngineConfig {
        display_name: name.clone(),
        port: cli.port,
        channel: current.channel(),
        passphrase: None,
        input_device: current.input_device.clone(),
        output_device: current.output_device.clone(),
        static_peers: cli.peers.clone(),
        half_duplex: current.half_duplex,
    }) {
        Ok(started) => started,
        Err(err) => {
            tracing::error!("couldn't start: {err}");
            eprintln!("takkie: {err}");
            return ExitCode::FAILURE;
        }
    };
    let local_ip = local_networks()
        .first()
        .map_or_else(|| "unknown".to_owned(), |net| net.ip.to_string());

    let mut app = ui::tui::App::new(name, local_ip, engine.port(), current.ptt_mode);
    let panel = match &logs {
        Some(logs) => {
            match (&logs.dir, &logs.problem) {
                (Some(dir), _) => app.note(format!("📝 Logs in {}", dir.display())),
                (None, Some(problem)) => app.note(format!("⚠️ No log files: {problem}")),
                (None, None) => {}
            }
            logs.panel.clone()
        }
        None => crossbeam_channel::never(),
    };
    if let Some(path) = &settings_path {
        app.note(format!("⚙️ Settings in {}", path.display()));
    }
    if let Some(note) = settings_note {
        app.note(note);
    }
    let result = ui::tui::run(app, &engine, &events, &panel);

    if let (Some(_), Some(path)) = (&saved, &settings_path) {
        current.channel = engine.snapshot().channel.get();
        if let Err(err) = settings::save(path, &current) {
            tracing::warn!("settings not saved: {err:#}");
            eprintln!("takkie: settings not saved: {err:#}");
        }
    }
    if let Err(err) = result {
        eprintln!("takkie: terminal error: {err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn list_devices() -> ExitCode {
    let devices = match Engine::list_devices() {
        Ok(devices) => devices,
        Err(err) => {
            eprintln!("takkie: {err}");
            return ExitCode::FAILURE;
        }
    };
    let print = || -> io::Result<()> {
        let mut out = io::stdout().lock();
        for (title, list) in [
            ("Microphones", &devices.inputs),
            ("Speakers", &devices.outputs),
        ] {
            writeln!(out, "{title}:")?;
            if list.is_empty() {
                writeln!(out, "  (none)")?;
            }
            for device in list {
                writeln!(out, "  {}", describe(device))?;
            }
        }
        Ok(())
    };
    match print() {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

fn describe(device: &DeviceInfo) -> String {
    if device.is_default {
        format!("{} (default)", device.name)
    } else {
        device.name.clone()
    }
}

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
