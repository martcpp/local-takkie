//! `settings.toml` in the platform config directory: what to remember
//! between runs.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use takkie_core::ChannelId;

/// How push-to-talk behaves; used once E9.3 lands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum PttMode {
    /// Hold if the terminal reports key releases, toggle otherwise.
    #[default]
    Auto,
    /// Talk while the key is held.
    Hold,
    /// Press once to talk, again to stop.
    Toggle,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub name: Option<String>,
    pub channel: u8,
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub ptt_mode: PttMode,
    pub half_duplex: bool,
    pub beeps: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            name: None,
            channel: ChannelId::MIN.get(),
            input_device: None,
            output_device: None,
            ptt_mode: PttMode::Auto,
            half_duplex: true,
            beeps: false,
        }
    }
}

impl Settings {
    pub fn channel(&self) -> ChannelId {
        ChannelId::try_from(self.channel).unwrap_or(ChannelId::MIN)
    }
}

/// What reading the file gave.
#[derive(Debug, PartialEq, Eq)]
pub enum Loaded {
    /// No file yet: defaults.
    Missing(Settings),
    /// The file, read fine.
    Read(Settings),
    /// The file couldn't be used, for this reason; it's left alone.
    Broken(String),
}

/// `settings.toml` in the platform config directory.
pub fn path() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "takkie")?;
    Some(dirs.config_dir().join("settings.toml"))
}

pub fn parse(text: &str) -> Result<Settings, String> {
    let settings: Settings = toml::from_str(text).map_err(|error| error.message().to_owned())?;
    ChannelId::try_from(settings.channel).map_err(|error| format!("channel: {error}"))?;
    Ok(settings)
}

pub fn load(path: &Path) -> Loaded {
    match fs::read_to_string(path) {
        Ok(text) => match parse(&text) {
            Ok(settings) => Loaded::Read(settings),
            Err(why) => Loaded::Broken(why),
        },
        Err(error) if error.kind() == ErrorKind::NotFound => Loaded::Missing(Settings::default()),
        Err(error) => Loaded::Broken(error.to_string()),
    }
}

/// Writes through a temporary file, so a crash never leaves half a file.
pub fn save(path: &Path, settings: &Settings) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("couldn't create {}", dir.display()))?;
    }
    let text = toml::to_string_pretty(settings).context("couldn't write the settings as TOML")?;
    let temporary = path.with_extension("toml.tmp");
    fs::write(&temporary, text)
        .with_context(|| format!("couldn't write {}", temporary.display()))?;
    fs::rename(&temporary, path).with_context(|| format!("couldn't replace {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir()
            .join(format!("takkie-settings-{name}-{unique}"))
            .join("settings.toml")
    }

    #[test]
    fn a_full_file_reads_back() {
        let text = r#"
            name = "Kitchen"
            channel = 4
            input_device = "Headset"
            output_device = "Speakers"
            ptt_mode = "toggle"
            half_duplex = false
            beeps = true
        "#;
        assert_eq!(
            parse(text),
            Ok(Settings {
                name: Some("Kitchen".into()),
                channel: 4,
                input_device: Some("Headset".into()),
                output_device: Some("Speakers".into()),
                ptt_mode: PttMode::Toggle,
                half_duplex: false,
                beeps: true,
            })
        );
    }

    #[test]
    fn missing_keys_get_defaults_and_unknown_keys_are_ignored() {
        let settings = parse("channel = 7\ntheme = \"dark\"\n").unwrap();
        assert_eq!(settings.channel().get(), 7);
        assert_eq!(settings.name, None);
        assert!(settings.half_duplex);
        assert!(!settings.beeps);
        assert_eq!(settings.ptt_mode, PttMode::Auto);
    }

    #[test]
    fn bad_files_are_errors() {
        assert!(parse("channel = 11").unwrap_err().starts_with("channel"));
        assert!(parse("channel = \"two\"").is_err());
        assert!(parse("ptt_mode = \"shout\"").is_err());
        assert!(parse("name = ").is_err());
    }

    #[test]
    fn saved_settings_survive_a_restart() {
        let path = scratch("roundtrip");
        assert_eq!(load(&path), Loaded::Missing(Settings::default()));
        let mine = Settings {
            name: Some("Bedroom".into()),
            channel: 9,
            output_device: Some("Headset".into()),
            ..Settings::default()
        };
        save(&path, &mine).unwrap();
        assert_eq!(load(&path), Loaded::Read(mine));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_broken_file_gives_a_reason() {
        let path = scratch("broken");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "channel = 99").unwrap();
        let Loaded::Broken(why) = load(&path) else {
            unreachable!("the file is broken");
        };
        assert!(why.contains("channel"));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn the_file_lives_in_a_takkie_folder() {
        let path = path().unwrap();
        assert!(path.ends_with("settings.toml"));
        assert!(path.components().any(|part| part.as_os_str() == "takkie"));
    }
}
