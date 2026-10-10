//! Terminal app for local-takkie.

use std::env;
use std::process::ExitCode;

use takkie_engine::net::address::local_networks;
use takkie_engine::{Engine, EngineConfig};

mod ui;

const USAGE: &str = "usage: takkie [name] [port]

  name  how others see you (default: this computer's name)
  port  UDP port to use (default: any free port)";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let (name, port) = match parse_args(&args, computer_name) {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("takkie: {err}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    let (engine, events) = match Engine::start(EngineConfig {
        display_name: name.clone(),
        port,
        ..EngineConfig::default()
    }) {
        Ok(started) => started,
        Err(err) => {
            eprintln!("takkie: {err}");
            return ExitCode::FAILURE;
        }
    };
    let local_ip = local_networks()
        .first()
        .map_or_else(|| "unknown".to_owned(), |net| net.ip.to_string());

    let app = ui::tui::App::new(name, local_ip, engine.port());
    if let Err(err) = ui::tui::run(app, &engine, &events) {
        eprintln!("takkie: terminal error: {err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// Reads `[name] [port]`. Missing values fall back to `default_name()` and
/// port 0, which lets the OS pick a free one.
fn parse_args(
    args: &[String],
    default_name: impl FnOnce() -> String,
) -> Result<(String, u16), String> {
    match args {
        [] => Ok((default_name(), 0)),
        [name] => Ok((name.clone(), 0)),
        [name, port] => port
            .parse()
            .map(|port| (name.clone(), port))
            .map_err(|_| format!("`{port}` isn't a port number (0 to 65535)")),
        _ => Err("too many arguments".to_owned()),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn no_args_uses_defaults() {
        assert_eq!(
            parse_args(&[], || "pc".to_owned()),
            Ok(("pc".to_owned(), 0))
        );
    }

    #[test]
    fn name_only_picks_any_port() {
        assert_eq!(
            parse_args(&args(&["alice"]), || unreachable!()),
            Ok(("alice".to_owned(), 0))
        );
    }

    #[test]
    fn name_and_port() {
        assert_eq!(
            parse_args(&args(&["alice", "5000"]), || unreachable!()),
            Ok(("alice".to_owned(), 5000))
        );
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
