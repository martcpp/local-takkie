//! Log files in the platform log directory, one per day, the last week
//! kept, plus warnings and errors for the in-app log panel.

use std::fmt::{self, Write as _};
use std::path::PathBuf;

use anyhow::{Context, anyhow};
use crossbeam_channel::{Receiver, Sender, bounded};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::filter::ParseError;
use tracing_subscriber::layer::{Context as LayerContext, Layer, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, fmt as format};

const KEEP_FILES: usize = 7;
const PANEL_LINES: usize = 256;

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

/// What logging ended up with.
pub struct Logging {
    /// Where today's file is, when files work.
    pub dir: Option<PathBuf>,
    /// Why there are no files, when they don't.
    pub problem: Option<String>,
    /// Warnings and errors, for the log panel.
    pub panel: Receiver<String>,
    _flush: Option<WorkerGuard>,
}

/// Sends logs to today's file and warnings to the panel. The returned value
/// must live until exit, or the last lines never reach the file.
pub fn init(filter: EnvFilter) -> anyhow::Result<Logging> {
    let (panel_layer, panel) = PanelLayer::new();
    let (files, dir, flush, problem) = match open_files() {
        Ok((writer, dir, flush)) => (
            Some(format::layer().with_writer(writer).with_ansi(false)),
            Some(dir),
            Some(flush),
            None,
        ),
        Err(error) => (None, None, None, Some(format!("{error:#}"))),
    };
    tracing_subscriber::registry()
        .with(filter)
        .with(files)
        .with(panel_layer)
        .try_init()
        .map_err(|error| anyhow!("couldn't start logging: {error}"))?;
    Ok(Logging {
        dir,
        problem,
        panel,
        _flush: flush,
    })
}

fn open_files() -> anyhow::Result<(NonBlocking, PathBuf, WorkerGuard)> {
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
    let (writer, flush) = tracing_appender::non_blocking(files);
    Ok((writer, dir, flush))
}

/// Passes warnings and errors on as one line each. Never blocks: when the
/// panel falls behind, lines are dropped.
pub struct PanelLayer {
    lines: Sender<String>,
}

impl PanelLayer {
    pub fn new() -> (Self, Receiver<String>) {
        let (lines, panel) = bounded(PANEL_LINES);
        (Self { lines }, panel)
    }
}

impl<S: Subscriber> Layer<S> for PanelLayer {
    fn on_event(&self, event: &Event<'_>, _: LayerContext<'_, S>) {
        let level = *event.metadata().level();
        if level > Level::WARN {
            return;
        }
        let mut text = Text::default();
        event.record(&mut text);
        let icon = if level == Level::ERROR {
            "❌"
        } else {
            "⚠️"
        };
        let _ = self
            .lines
            .try_send(format!("{icon} {}{}", text.message, text.fields));
    }
}

#[derive(Default)]
struct Text {
    message: String,
    fields: String,
}

impl Visit for Text {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_owned();
        } else {
            let _ = write!(self.fields, " {}={value}", field.name());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            let _ = write!(self.fields, " {}={value:?}", field.name());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warnings_and_errors_reach_the_panel_and_info_does_not() {
        let (layer, panel) = PanelLayer::new();
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("engine started");
            tracing::warn!(port = 5000, "port is busy");
            tracing::error!(device = "Headset", "mic stopped");
        });
        let lines: Vec<String> = panel.try_iter().collect();
        assert_eq!(
            lines,
            ["⚠️ port is busy port=5000", "❌ mic stopped device=Headset"]
        );
    }

    #[test]
    fn a_full_panel_drops_lines_instead_of_blocking() {
        let (layer, panel) = PanelLayer::new();
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            for i in 0..PANEL_LINES + 10 {
                tracing::warn!("line {i}");
            }
        });
        assert_eq!(panel.try_iter().count(), PANEL_LINES);
    }

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
