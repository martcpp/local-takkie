//! Picks a stream config the device supports. 48 kHz f32 needs no
//! resampling or conversion, so it wins when offered; otherwise the device
//! default is used.

use std::fmt;

use cpal::traits::DeviceTrait;
use cpal::{
    DefaultStreamConfigError, SampleFormat, SupportedStreamConfig, SupportedStreamConfigRange,
    SupportedStreamConfigsError,
};
use thiserror::Error;

/// Opus's native rate.
pub const PREFERRED_RATE: u32 = 48_000;

/// The config a stream runs with, for showing in the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamSettings {
    /// Samples per second.
    pub sample_rate: u32,
    /// Interleaved channels.
    pub channels: u16,
    /// Sample type.
    pub format: SampleFormat,
}

impl From<&SupportedStreamConfig> for StreamSettings {
    fn from(config: &SupportedStreamConfig) -> Self {
        Self {
            sample_rate: config.sample_rate(),
            channels: config.channels(),
            format: config.sample_format(),
        }
    }
}

impl fmt::Display for StreamSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} Hz, {} ch, {}",
            self.sample_rate, self.channels, self.format
        )
    }
}

/// Why a device's configs couldn't be read.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Listing supported configs failed.
    #[error("couldn't read the device's stream configs: {0}")]
    Supported(#[from] SupportedStreamConfigsError),
    /// The device has no default config.
    #[error("the device has no default stream config: {0}")]
    Default(#[from] DefaultStreamConfigError),
}

/// 48 kHz f32 if any range offers it, keeping the default's channel count
/// when possible; otherwise `default`.
pub fn choose_config(
    supported: impl IntoIterator<Item = SupportedStreamConfigRange>,
    default: SupportedStreamConfig,
) -> SupportedStreamConfig {
    supported
        .into_iter()
        .filter(|range| range.sample_format() == SampleFormat::F32)
        .filter_map(|range| range.try_with_sample_rate(PREFERRED_RATE))
        .min_by_key(|config| (config.channels() != default.channels(), config.channels()))
        .unwrap_or(default)
}

/// The config to capture from `device` with.
///
/// # Errors
/// [`ConfigError`] if the device can't report its configs.
pub fn input_config(device: &cpal::Device) -> Result<SupportedStreamConfig, ConfigError> {
    let default = device.default_input_config()?;
    Ok(choose_config(device.supported_input_configs()?, default))
}

/// The config to play to `device` with.
///
/// # Errors
/// [`ConfigError`] if the device can't report its configs.
pub fn output_config(device: &cpal::Device) -> Result<SupportedStreamConfig, ConfigError> {
    let default = device.default_output_config()?;
    Ok(choose_config(device.supported_output_configs()?, default))
}

#[cfg(test)]
mod tests {
    use cpal::SupportedBufferSize;

    use super::*;

    fn range(
        channels: u16,
        min: u32,
        max: u32,
        format: SampleFormat,
    ) -> SupportedStreamConfigRange {
        SupportedStreamConfigRange::new(channels, min, max, SupportedBufferSize::Unknown, format)
    }

    fn default(channels: u16, rate: u32, format: SampleFormat) -> SupportedStreamConfig {
        SupportedStreamConfig::new(channels, rate, SupportedBufferSize::Unknown, format)
    }

    fn settings(config: &SupportedStreamConfig) -> StreamSettings {
        StreamSettings::from(config)
    }

    #[test]
    fn picks_48k_f32_with_the_default_channel_count() {
        let chosen = choose_config(
            [
                range(1, 48_000, 48_000, SampleFormat::F32),
                range(2, 48_000, 48_000, SampleFormat::F32),
            ],
            default(2, 44_100, SampleFormat::I16),
        );
        assert_eq!(
            settings(&chosen),
            StreamSettings {
                sample_rate: 48_000,
                channels: 2,
                format: SampleFormat::F32
            }
        );
    }

    #[test]
    fn picks_48k_from_inside_a_range() {
        let chosen = choose_config(
            [range(2, 8_000, 96_000, SampleFormat::F32)],
            default(2, 44_100, SampleFormat::F32),
        );
        assert_eq!(chosen.sample_rate(), 48_000);
    }

    #[test]
    fn falls_back_to_fewest_channels_when_the_default_count_is_missing() {
        let chosen = choose_config(
            [
                range(4, 48_000, 48_000, SampleFormat::F32),
                range(1, 48_000, 48_000, SampleFormat::F32),
            ],
            default(2, 44_100, SampleFormat::F32),
        );
        assert_eq!(chosen.channels(), 1);
    }

    #[test]
    fn uses_the_default_without_48k() {
        let fallback = default(2, 44_100, SampleFormat::F32);
        let chosen = choose_config(
            [range(2, 44_100, 44_100, SampleFormat::F32)],
            fallback.clone(),
        );
        assert_eq!(chosen, fallback);
    }

    #[test]
    fn uses_the_default_without_f32() {
        let fallback = default(1, 48_000, SampleFormat::I16);
        let chosen = choose_config(
            [range(1, 48_000, 48_000, SampleFormat::I16)],
            fallback.clone(),
        );
        assert_eq!(chosen, fallback);
    }

    #[test]
    fn uses_the_default_when_nothing_is_listed() {
        let fallback = default(2, 16_000, SampleFormat::U16);
        assert_eq!(choose_config([], fallback.clone()), fallback);
    }

    #[test]
    fn settings_display_for_the_ui() {
        let config = default(2, 44_100, SampleFormat::I16);
        assert_eq!(settings(&config).to_string(), "44100 Hz, 2 ch, i16");
    }
}
