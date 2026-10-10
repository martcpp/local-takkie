//! Terminal app for local-takkie.

use std::env;
use std::process::ExitCode;

use takkie_engine::net::address::local_networks;
use takkie_engine::{Engine, EngineConfig};

mod logging;
mod settings;
mod ui;

const USAGE: &str = "usage: takkie [--log-level LEVEL] [name] [port]

  name         how others see you (default: the settings file, else this
               computer's name)
  port         UDP port to use (default: any free port)
  --log-level  error, warn, info, debug or trace, or per crate like
               info,takkie_engine=debug (default: RUST_LOG, else info)";

#[derive(Debug, PartialEq, Eq)]
struct Args {
    name: Option<String>,
    port: u16,
    log_level: Option<String>,
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let Args {
        name,
        port,
        log_level,
    } = match parse_args(&args) {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("takkie: {err}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    let filter = match logging::filter(log_level.as_deref(), env::var("RUST_LOG").ok().as_deref()) {
        Ok(filter) => filter,
        Err(err) => {
            eprintln!("takkie: bad log level: {err}\n\n{USAGE}");
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

    let settings_path = settings::path();
    let loaded = settings_path.as_deref().map(settings::load);
    let (mut saved, settings_note) = match loaded {
        Some(settings::Loaded::Missing(saved)) => (Some(saved), None),
        Some(settings::Loaded::Read(saved)) => (Some(saved), None),
        Some(settings::Loaded::Broken(why)) => {
            tracing::warn!("settings file ignored: {why}");
            let note = format!("⚠️ Settings file ignored ({why}), using defaults");
            (None, Some(note))
        }
        None => (
            None,
            Some("⚠️ No config directory, settings won't be saved".to_owned()),
        ),
    };
    let current = saved.clone().unwrap_or_default();
    let cli_name = name.is_some();
    let name = name
        .or_else(|| current.name.clone())
        .unwrap_or_else(computer_name);

    let (engine, events) = match Engine::start(EngineConfig {
        display_name: name.clone(),
        port,
        channel: current.channel(),
        input_device: current.input_device.clone(),
        output_device: current.output_device.clone(),
        half_duplex: current.half_duplex,
        ..EngineConfig::default()
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

    let mut app = ui::tui::App::new(name.clone(), local_ip, engine.port());
    if let Some((dir, _)) = &logs {
        app.note(format!("📝 Logs in {}", dir.display()));
    }
    if let Some(path) = &settings_path {
        app.note(format!("⚙️ Settings in {}", path.display()));
    }
    if let Some(note) = settings_note {
        app.note(note);
    }
    let result = ui::tui::run(app, &engine, &events);

    if let (Some(saved), Some(path)) = (&mut saved, &settings_path) {
        if cli_name {
            saved.name = Some(name);
        }
        saved.channel = engine.snapshot().channel.get();
        if let Err(err) = settings::save(path, saved) {
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

/// Reads `[--log-level LEVEL] [name] [port]`. A missing port is 0, which
/// lets the OS pick a free one.
fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut log_level = None;
    let mut rest = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if let Some(level) = arg.strip_prefix("--log-level=") {
            log_level = Some(level.to_owned());
        } else if arg == "--log-level" {
            log_level = Some(args.next().ok_or("--log-level needs a level")?.clone());
        } else {
            rest.push(arg);
        }
    }
    let (name, port) = match rest.as_slice() {
        [] => (None, 0),
        [name] => (Some((*name).clone()), 0),
        [name, port] => (
            Some((*name).clone()),
            port.parse()
                .map_err(|_| format!("`{port}` isn't a port number (0 to 65535)"))?,
        ),
        _ => return Err("too many arguments".to_owned()),
    };
    Ok(Args {
        name,
        port,
        log_level,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_owned()).collect()
    }

    fn parsed(name: Option<&str>, port: u16, log_level: Option<&str>) -> Result<Args, String> {
        Ok(Args {
            name: name.map(str::to_owned),
            port,
            log_level: log_level.map(str::to_owned),
        })
    }

    #[test]
    fn no_args_uses_defaults() {
        assert_eq!(parse_args(&[]), parsed(None, 0, None));
    }

    #[test]
    fn name_only_picks_any_port() {
        assert_eq!(
            parse_args(&args(&["alice"])),
            parsed(Some("alice"), 0, None)
        );
    }

    #[test]
    fn name_and_port() {
        assert_eq!(
            parse_args(&args(&["alice", "5000"])),
            parsed(Some("alice"), 5000, None)
        );
    }

    #[test]
    fn log_level_goes_anywhere_in_either_form() {
        assert_eq!(
            parse_args(&args(&["--log-level", "debug", "alice"])),
            parsed(Some("alice"), 0, Some("debug"))
        );
        assert_eq!(
            parse_args(&args(&["alice", "5000", "--log-level=trace"])),
            parsed(Some("alice"), 5000, Some("trace"))
        );
        assert!(parse_args(&args(&["--log-level"])).is_err());
    }

    #[test]
    fn bad_port_is_an_error() {
        assert!(parse_args(&args(&["alice", "nope"])).is_err());
        assert!(parse_args(&args(&["alice", "70000"])).is_err());
    }

    #[test]
    fn too_many_args_is_an_error() {
        assert!(parse_args(&args(&["a", "1", "x"])).is_err());
    }
}
