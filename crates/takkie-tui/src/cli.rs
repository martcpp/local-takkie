//! Command-line flags.

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;
use takkie_core::ChannelId;

use crate::settings::{PttMode, Settings};

/// Walkie-talkie for your local network. Hold SPACE to talk.
///
/// Flags override the settings file for this run. The name, channel, devices
/// and PTT mode you ran with are remembered when you quit.
///
/// To start on a private channel, set the TAKKIE_PASSPHRASE environment
/// variable, or press P in the app. There is no flag for it on purpose: it
/// would end up in your shell history.
#[derive(Debug, Parser)]
#[command(name = "takkie", version)]
pub struct Cli {
    /// How others see you [default: the settings file, else this computer's name]
    #[arg(short, long)]
    pub name: Option<String>,

    /// Channel to join, 1 to 10
    #[arg(short, long, value_parser = channel)]
    pub channel: Option<ChannelId>,

    /// UDP port; 0 picks any free one
    #[arg(short, long, default_value_t = 0)]
    pub port: u16,

    /// Microphone to use; part of its name is enough (see --list-devices)
    #[arg(long, value_name = "NAME")]
    pub input_device: Option<String>,

    /// Speaker to use; part of its name is enough
    #[arg(long, value_name = "NAME")]
    pub output_device: Option<String>,

    /// List microphones and speakers, then exit
    #[arg(long)]
    pub list_devices: bool,

    /// Greet this address directly, for networks that block mDNS (repeatable)
    #[arg(long = "peer", value_name = "IP:PORT")]
    pub peers: Vec<SocketAddr>,

    /// How push-to-talk works
    #[arg(long, value_enum)]
    pub ptt_mode: Option<PttMode>,

    /// error, warn, info, debug or trace, or per crate like
    /// info,takkie_engine=debug [default: RUST_LOG, else info]
    #[arg(long, value_name = "LEVEL")]
    pub log_level: Option<String>,

    /// Use this settings file instead of the usual one
    #[arg(long, value_name = "PATH", env = "TAKKIE_CONFIG")]
    pub config: Option<PathBuf>,
}

fn channel(text: &str) -> Result<ChannelId, String> {
    let number: u8 = text
        .parse()
        .map_err(|_| format!("`{text}` isn't a channel number (1 to 10)"))?;
    ChannelId::try_from(number).map_err(|error| error.to_string())
}

impl Cli {
    /// `settings` with this run's flags on top.
    pub fn apply(&self, mut settings: Settings) -> Settings {
        if let Some(name) = &self.name {
            settings.name = Some(name.clone());
        }
        if let Some(channel) = self.channel {
            settings.channel = channel.get();
        }
        if let Some(device) = &self.input_device {
            settings.input_device = Some(device.clone());
        }
        if let Some(device) = &self.output_device {
            settings.output_device = Some(device.clone());
        }
        if let Some(mode) = self.ptt_mode {
            settings.ptt_mode = mode;
        }
        settings
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;
    use clap::error::ErrorKind;

    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("takkie").chain(args.iter().copied()))
    }

    #[test]
    fn the_flag_definitions_are_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn every_flag_parses() {
        let cli = parse(&[
            "--name",
            "Kitchen",
            "--channel",
            "4",
            "--port",
            "5000",
            "--input-device",
            "Headset",
            "--output-device",
            "Speakers",
            "--peer",
            "192.168.1.5:40000",
            "--peer",
            "10.0.0.2:5000",
            "--ptt-mode",
            "toggle",
            "--log-level",
            "debug",
            "--config",
            "kitchen.toml",
        ])
        .unwrap();
        assert_eq!(cli.name.as_deref(), Some("Kitchen"));
        assert_eq!(cli.channel.map(ChannelId::get), Some(4));
        assert_eq!(cli.port, 5000);
        assert_eq!(cli.peers.len(), 2);
        assert_eq!(cli.ptt_mode, Some(PttMode::Toggle));
        assert_eq!(cli.log_level.as_deref(), Some("debug"));
        assert_eq!(cli.config, Some(PathBuf::from("kitchen.toml")));
        assert!(!cli.list_devices);
    }

    #[test]
    fn nothing_given_means_nothing_overridden() {
        let cli = parse(&[]).unwrap();
        assert_eq!(cli.port, 0);
        assert!(cli.peers.is_empty());
        let file = Settings {
            name: Some("Bedroom".into()),
            channel: 7,
            ..Settings::default()
        };
        assert_eq!(cli.apply(file.clone()), file);
    }

    #[test]
    fn flags_beat_the_settings_file() {
        let cli = parse(&["-n", "Kitchen", "-c", "2", "--input-device", "USB"]).unwrap();
        let file = Settings {
            name: Some("Bedroom".into()),
            channel: 7,
            output_device: Some("Speakers".into()),
            ..Settings::default()
        };
        let applied = cli.apply(file);
        assert_eq!(applied.name.as_deref(), Some("Kitchen"));
        assert_eq!(applied.channel, 2);
        assert_eq!(applied.input_device.as_deref(), Some("USB"));
        assert_eq!(applied.output_device.as_deref(), Some("Speakers"));
    }

    #[test]
    fn bad_values_are_rejected_with_a_reason() {
        for args in [
            &["--channel", "11"][..],
            &["--channel", "two"],
            &["--port", "70000"],
            &["--peer", "kitchen"],
            &["--ptt-mode", "shout"],
            &["alice"],
        ] {
            assert!(parse(args).is_err(), "{args:?} was accepted");
        }
        let error = parse(&["--channel", "11"]).unwrap_err().to_string();
        assert!(error.contains("1"), "{error}");
    }

    #[test]
    fn version_and_help_are_answered() {
        assert_eq!(
            parse(&["--version"]).unwrap_err().kind(),
            ErrorKind::DisplayVersion
        );
        let help = Cli::command().render_long_help().to_string();
        for flag in [
            "--name",
            "--channel",
            "--port",
            "--input-device",
            "--output-device",
            "--list-devices",
            "--peer",
            "--ptt-mode",
            "--log-level",
            "--config",
        ] {
            assert!(help.contains(flag), "--help is missing {flag}");
        }
    }
}
