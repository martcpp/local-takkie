//! Terminal app for local-takkie.

use std::env;
use std::process::ExitCode;

use takkie_engine::net::address::local_networks;
use takkie_engine::{Engine, EngineConfig};

mod logging;
mod ui;

const USAGE: &str = "usage: takkie [--log-level LEVEL] [name] [port]

  name         how others see you (default: this computer's name)
  port         UDP port to use (default: any free port)
  --log-level  error, warn, info, debug or trace, or per crate like
               info,takkie_engine=debug (default: RUST_LOG, else info)";

#[derive(Debug, PartialEq, Eq)]
struct Args {
    name: String,
    port: u16,
    log_level: Option<String>,
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let Args {
        name,
        port,
        log_level,
    } = match parse_args(&args, computer_name) {
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

    let (engine, events) = match Engine::start(EngineConfig {
        display_name: name.clone(),
        port,
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

    let mut app = ui::tui::App::new(name, local_ip, engine.port());
    if let Some((dir, _)) = &logs {
        app.note(format!("📝 Logs in {}", dir.display()));
    }
    if let Err(err) = ui::tui::run(app, &engine, &events) {
        eprintln!("takkie: terminal error: {err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// Reads `[--log-level LEVEL] [name] [port]`. Missing values fall back to
/// `default_name()` and port 0, which lets the OS pick a free one.
fn parse_args(args: &[String], default_name: impl FnOnce() -> String) -> Result<Args, String> {
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
        [] => (default_name(), 0),
        [name] => ((*name).clone(), 0),
        [name, port] => (
            (*name).clone(),
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

    fn parsed(name: &str, port: u16, log_level: Option<&str>) -> Result<Args, String> {
        Ok(Args {
            name: name.to_owned(),
            port,
            log_level: log_level.map(str::to_owned),
        })
    }

    #[test]
    fn no_args_uses_defaults() {
        assert_eq!(parse_args(&[], || "pc".to_owned()), parsed("pc", 0, None));
    }

    #[test]
    fn name_only_picks_any_port() {
        assert_eq!(
            parse_args(&args(&["alice"]), || unreachable!()),
            parsed("alice", 0, None)
        );
    }

    #[test]
    fn name_and_port() {
        assert_eq!(
            parse_args(&args(&["alice", "5000"]), || unreachable!()),
            parsed("alice", 5000, None)
        );
    }

    #[test]
    fn log_level_goes_anywhere_in_either_form() {
        assert_eq!(
            parse_args(&args(&["--log-level", "debug", "alice"]), || unreachable!()),
            parsed("alice", 0, Some("debug"))
        );
        assert_eq!(
            parse_args(&args(&["alice", "5000", "--log-level=trace"]), || {
                unreachable!()
            }),
            parsed("alice", 5000, Some("trace"))
        );
        assert!(parse_args(&args(&["--log-level"]), || "pc".to_owned()).is_err());
    }

    #[test]
    fn bad_port_is_an_error() {
        assert!(parse_args(&args(&["alice", "nope"]), || unreachable!()).is_err());
        assert!(parse_args(&args(&["alice", "70000"]), || unreachable!()).is_err());
    }

    #[test]
    fn too_many_args_is_an_error() {
        assert!(parse_args(&args(&["a", "1", "x"]), || unreachable!()).is_err());
    }
}
