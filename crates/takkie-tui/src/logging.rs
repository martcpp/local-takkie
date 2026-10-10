//! Log files in the platform log directory, one per day, the last week kept.

use std::path::PathBuf;

use anyhow::{Context, anyhow};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::ParseError;

const KEEP_FILES: usize = 7;

/// `~/Library/Logs/takkie` on macOS.
#[cfg(target_os = "macos")]
pub fn log_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|dirs| dirs.home_dir().join("Library/Logs/takkie"))
}

/// The state directory on Linux, local app data on Windows, plus `logs`.
#[cfg(not(target_os = "macos"))]
pub fn log_dir() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "takkie")?;
    let base = dirs.state_dir().unwrap_or_else(|| dirs.data_local_dir());
    Some(base.join("logs"))
}

/// `--log-level` wins, then `RUST_LOG`, then `info`.
pub fn filter(flag: Option<&str>, env: Option<&str>) -> Result<EnvFilter, ParseError> {
    EnvFilter::try_new(given(flag).or(given(env)).unwrap_or("info"))
}

fn given(level: Option<&str>) -> Option<&str> {
    level.filter(|level| !level.trim().is_empty())
}

/// Sends logs to today's file. Keep the guard until exit, or the last lines
/// are lost.
pub fn init(filter: EnvFilter) -> anyhow::Result<(PathBuf, WorkerGuard)> {
    let dir = log_dir().context("no home directory to keep logs in")?;
    // The appender prunes old files first and warns on stderr if the folder is missing.
    std::fs::create_dir_all(&dir).with_context(|| format!("couldn't create {}", dir.display()))?;
    let files = Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix("takkie")
        .filename_suffix("log")
        .max_log_files(KEEP_FILES)
        .build(&dir)
        .with_context(|| format!("couldn't create log files in {}", dir.display()))?;
    let (writer, guard) = tracing_appender::non_blocking(files);
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter(filter)
        .try_init()
        .map_err(|error| anyhow!("couldn't start logging: {error}"))?;
    Ok((dir, guard))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flag_beats_rust_log_which_beats_the_default() {
        assert_eq!(
            filter(Some("debug"), Some("warn")).unwrap().to_string(),
            "debug"
        );
        assert_eq!(filter(None, Some("warn")).unwrap().to_string(), "warn");
        assert_eq!(filter(None, None).unwrap().to_string(), "info");
        assert_eq!(filter(Some(" "), Some("")).unwrap().to_string(), "info");
    }

    #[test]
    fn per_crate_levels_work_and_nonsense_is_an_error() {
        assert_eq!(
            filter(Some("info,takkie_engine=debug"), None)
                .unwrap()
                .to_string(),
            "takkie_engine=debug,info"
        );
        assert!(filter(Some("loud=very"), None).is_err());
    }

    #[test]
    fn logs_live_in_a_takkie_folder() {
        let dir = log_dir().unwrap();
        assert!(dir.components().any(|part| part.as_os_str() == "takkie"));
    }
}
