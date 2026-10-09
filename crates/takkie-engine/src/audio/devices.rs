//! What audio devices exist, for `--list-devices` and device pickers.

use std::collections::BTreeSet;
use std::fmt;

use cpal::traits::{DeviceTrait, HostTrait};
use cpal::{DevicesError, SampleFormat, SupportedStreamConfigRange};
use thiserror::Error;

/// One device and what it supports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Name, as used to pick it.
    pub name: String,
    /// The system default.
    pub is_default: bool,
    /// Lowest and highest sample rate.
    pub rates: Option<(u32, u32)>,
    /// Most channels offered.
    pub max_channels: u16,
    /// Sample formats offered.
    pub formats: Vec<SampleFormat>,
}

/// Every input and output device.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceList {
    /// Microphones.
    pub inputs: Vec<DeviceInfo>,
    /// Speakers.
    pub outputs: Vec<DeviceInfo>,
}

/// Why devices couldn't be listed.
#[derive(Debug, Error)]
#[error("couldn't list audio devices: {0}")]
pub struct DeviceError(#[from] DevicesError);

/// Lists the default host's devices.
///
/// # Errors
/// [`DeviceError`] if the audio system can't be asked.
pub fn list_devices() -> Result<DeviceList, DeviceError> {
    let host = cpal::default_host();
    let default_in = host.default_input_device().and_then(|d| d.id().ok());
    let default_out = host.default_output_device().and_then(|d| d.id().ok());

    let inputs = host
        .input_devices()?
        .map(|device| {
            let configs = device.supported_input_configs().into_iter().flatten();
            info(&device, default_in.as_ref(), configs)
        })
        .collect();
    let outputs = host
        .output_devices()?
        .map(|device| {
            let configs = device.supported_output_configs().into_iter().flatten();
            info(&device, default_out.as_ref(), configs)
        })
        .collect();
    Ok(DeviceList { inputs, outputs })
}

fn info(
    device: &cpal::Device,
    default: Option<&cpal::DeviceId>,
    configs: impl Iterator<Item = SupportedStreamConfigRange>,
) -> DeviceInfo {
    let name = device
        .description()
        .map(|d| d.name().to_string())
        .unwrap_or_else(|_| "unknown device".into());
    let is_default = default.is_some() && device.id().ok().as_ref() == default;
    summarize(name, is_default, configs)
}

fn summarize(
    name: String,
    is_default: bool,
    configs: impl Iterator<Item = SupportedStreamConfigRange>,
) -> DeviceInfo {
    let mut rates: Option<(u32, u32)> = None;
    let mut max_channels = 0;
    let mut formats = BTreeSet::new();
    for config in configs {
        let (low, high) = (config.min_sample_rate(), config.max_sample_rate());
        rates = Some(rates.map_or((low, high), |(a, b)| (a.min(low), b.max(high))));
        max_channels = max_channels.max(config.channels());
        formats.insert(config.sample_format());
    }
    DeviceInfo {
        name,
        is_default,
        rates,
        max_channels,
        formats: formats.into_iter().collect(),
    }
}

impl fmt::Display for DeviceInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)?;
        if self.is_default {
            write!(f, " [default]")?;
        }
        match self.rates {
            Some((low, high)) if low == high => write!(f, ": {low} Hz")?,
            Some((low, high)) => write!(f, ": {low}-{high} Hz")?,
            None => return write!(f, ": no configs reported"),
        }
        write!(f, ", up to {} ch,", self.max_channels)?;
        for format in &self.formats {
            write!(f, " {format}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use cpal::SupportedBufferSize;

    use super::*;

    fn range(
        channels: u16,
        low: u32,
        high: u32,
        format: SampleFormat,
    ) -> SupportedStreamConfigRange {
        SupportedStreamConfigRange::new(channels, low, high, SupportedBufferSize::Unknown, format)
    }

    #[test]
    fn summary_merges_every_config() {
        let info = summarize(
            "Headset".into(),
            true,
            [
                range(1, 16_000, 16_000, SampleFormat::I16),
                range(2, 44_100, 48_000, SampleFormat::F32),
                range(2, 8_000, 48_000, SampleFormat::I16),
            ]
            .into_iter(),
        );
        assert_eq!(info.rates, Some((8_000, 48_000)));
        assert_eq!(info.max_channels, 2);
        assert_eq!(info.formats, [SampleFormat::I16, SampleFormat::F32]);
        assert_eq!(
            info.to_string(),
            "Headset [default]: 8000-48000 Hz, up to 2 ch, i16 f32"
        );
    }

    #[test]
    fn a_device_without_configs_says_so() {
        let info = summarize("Ghost".into(), false, std::iter::empty());
        assert_eq!(info.to_string(), "Ghost: no configs reported");
    }

    #[test]
    fn a_single_rate_is_shown_once() {
        let info = summarize(
            "Mic".into(),
            false,
            [range(1, 48_000, 48_000, SampleFormat::F32)].into_iter(),
        );
        assert_eq!(info.to_string(), "Mic: 48000 Hz, up to 1 ch, f32");
    }
}
