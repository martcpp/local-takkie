//! Small audio helpers for the engine's real-time paths. Samples are `f32` in
//! [-1, 1], multi-channel buffers are interleaved, and the caller owns every
//! buffer, so nothing here allocates.

/// Averages interleaved frames into mono and returns how many samples it
/// wrote. A trailing partial frame is ignored.
pub fn downmix_to_mono(input: &[f32], channels: usize, out: &mut [f32]) -> usize {
    if channels == 0 {
        return 0;
    }
    let scale = 1.0 / channels as f32;
    let mut written = 0;
    for (frame, sample) in input.chunks_exact(channels).zip(out.iter_mut()) {
        *sample = frame.iter().sum::<f32>() * scale;
        written += 1;
    }
    written
}

/// Copies each mono sample to every channel and returns how many frames it
/// wrote.
pub fn upmix_from_mono(input: &[f32], channels: usize, out: &mut [f32]) -> usize {
    if channels == 0 {
        return 0;
    }
    let mut written = 0;
    for (&sample, frame) in input.iter().zip(out.chunks_exact_mut(channels)) {
        frame.fill(sample);
        written += 1;
    }
    written
}

/// Multiplies every sample by `gain`.
pub fn apply_gain(samples: &mut [f32], gain: f32) {
    samples.iter_mut().for_each(|sample| *sample *= gain);
}

/// Adds `src` into `acc`, sample by sample, up to the shorter length.
pub fn mix_into(acc: &mut [f32], src: &[f32]) {
    acc.iter_mut()
        .zip(src)
        .for_each(|(sum, sample)| *sum += sample);
}

const KNEE: f32 = 0.8;

/// Keeps samples inside [-1, 1]. Anything up to 0.8 is untouched; louder
/// samples curve smoothly towards 1 instead of clipping hard. NaN becomes 0.
pub fn soft_limit(samples: &mut [f32]) {
    for sample in samples {
        if sample.is_nan() {
            *sample = 0.0;
        } else if sample.abs() > KNEE {
            let over = (sample.abs() - KNEE) / (1.0 - KNEE);
            *sample = sample.signum() * (KNEE + (1.0 - KNEE) * over.tanh());
        }
    }
}

/// Loudness of one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Level {
    /// Root mean square, for a meter.
    pub rms: f32,
    /// Largest absolute sample.
    pub peak: f32,
}

impl Level {
    /// Measures `samples`; an empty frame is silent.
    #[must_use]
    pub fn of(samples: &[f32]) -> Self {
        if samples.is_empty() {
            return Self::default();
        }
        let squares: f32 = samples.iter().map(|sample| sample * sample).sum();
        Self {
            rms: (squares / samples.len() as f32).sqrt(),
            peak: samples
                .iter()
                .fold(0.0, |peak, sample| peak.max(sample.abs())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_downmix_is_a_copy() {
        let mut out = [0.0; 3];
        assert_eq!(downmix_to_mono(&[0.1, -0.2, 0.3], 1, &mut out), 3);
        assert_eq!(out, [0.1, -0.2, 0.3]);
    }

    #[test]
    fn stereo_downmix_averages_left_and_right() {
        let mut out = [0.0; 2];
        assert_eq!(downmix_to_mono(&[1.0, 0.0, 0.5, -0.5], 2, &mut out), 2);
        assert_eq!(out, [0.5, 0.0]);
    }

    #[test]
    fn four_and_six_channel_downmix() {
        let mut out = [0.0; 1];
        assert_eq!(downmix_to_mono(&[0.4, 0.4, 0.0, 0.0], 4, &mut out), 1);
        assert_eq!(out, [0.2]);
        assert_eq!(downmix_to_mono(&[0.6; 6], 6, &mut out), 1);
        assert!((out[0] - 0.6).abs() < 1e-6);
    }

    #[test]
    fn downmix_stops_at_the_shorter_buffer() {
        let mut out = [9.0; 4];
        assert_eq!(downmix_to_mono(&[1.0, 1.0, 1.0], 2, &mut out), 1);
        assert_eq!(out, [1.0, 9.0, 9.0, 9.0]);

        let mut short = [0.0; 1];
        assert_eq!(downmix_to_mono(&[0.2; 8], 2, &mut short), 1);
    }

    #[test]
    fn upmix_copies_to_every_channel() {
        let mut stereo = [0.0; 4];
        assert_eq!(upmix_from_mono(&[0.1, -0.3], 2, &mut stereo), 2);
        assert_eq!(stereo, [0.1, 0.1, -0.3, -0.3]);

        let mut quad = [0.0; 4];
        assert_eq!(upmix_from_mono(&[0.5], 4, &mut quad), 1);
        assert_eq!(quad, [0.5; 4]);

        let mut six = [0.0; 12];
        assert_eq!(upmix_from_mono(&[0.2, 0.4], 6, &mut six), 2);
        assert_eq!(six[..6], [0.2; 6]);
        assert_eq!(six[6..], [0.4; 6]);
    }

    #[test]
    fn upmix_stops_at_the_shorter_buffer() {
        let mut out = [9.0; 5];
        assert_eq!(upmix_from_mono(&[0.1, 0.2, 0.3], 2, &mut out), 2);
        assert_eq!(out, [0.1, 0.1, 0.2, 0.2, 9.0]);
    }

    #[test]
    fn zero_channels_writes_nothing() {
        let mut out = [9.0; 2];
        assert_eq!(downmix_to_mono(&[1.0], 0, &mut out), 0);
        assert_eq!(upmix_from_mono(&[1.0], 0, &mut out), 0);
        assert_eq!(out, [9.0; 2]);
    }

    #[test]
    fn gain_scales_every_sample() {
        let mut samples = [0.5, -0.25, 0.0];
        apply_gain(&mut samples, 0.5);
        assert_eq!(samples, [0.25, -0.125, 0.0]);
        apply_gain(&mut samples, 0.0);
        assert_eq!(samples, [0.0, -0.0, 0.0]);
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn mix_adds_sample_by_sample() {
        let mut acc = [0.1, 0.2, 0.3];
        mix_into(&mut acc, &[0.1, -0.2]);
        assert!(close(acc[0], 0.2));
        assert!(close(acc[1], 0.0));
        assert!(close(acc[2], 0.3));
    }

    #[test]
    fn limiter_leaves_normal_levels_alone() {
        let mut samples = [0.0, 0.5, -0.8, 0.8];
        soft_limit(&mut samples);
        assert_eq!(samples, [0.0, 0.5, -0.8, 0.8]);
    }

    #[test]
    fn limiter_keeps_loud_samples_inside_one_and_in_order() {
        let mut samples = [
            0.9,
            1.0,
            1.5,
            3.0,
            100.0,
            -2.0,
            f32::INFINITY,
            f32::NEG_INFINITY,
        ];
        soft_limit(&mut samples);
        assert!(samples.iter().all(|s| (-1.0..=1.0).contains(s)));
        assert!(samples[0] > KNEE && samples[0] < samples[1]);
        assert!(samples[1] < samples[2] && samples[2] < samples[3]);
        assert!(samples[5] < -KNEE && samples[5] > -1.0);
        assert_eq!(samples[6], 1.0);
        assert_eq!(samples[7], -1.0);
    }

    #[test]
    fn limiter_is_symmetric() {
        let mut up = [0.95, 1.7];
        let mut down = [-0.95, -1.7];
        soft_limit(&mut up);
        soft_limit(&mut down);
        assert_eq!(up, [-down[0], -down[1]]);
    }

    #[test]
    fn limiter_turns_nan_into_silence() {
        let mut samples = [f32::NAN, 0.5];
        soft_limit(&mut samples);
        assert_eq!(samples, [0.0, 0.5]);
    }

    #[test]
    fn several_loud_talkers_mixed_stay_within_one() {
        let talker: Vec<f32> = (0..480).map(|i| 0.9 * (i as f32 * 0.07).sin()).collect();
        let mut mix = [0.0; 480];
        for _ in 0..4 {
            mix_into(&mut mix, &talker);
        }
        assert!(Level::of(&mix).peak > 3.0);
        soft_limit(&mut mix);
        assert!(mix.iter().all(|s| (-1.0..=1.0).contains(s)));
    }

    #[test]
    fn level_of_a_sine_wave() {
        let sine: Vec<f32> = (0..4800)
            .map(|i| 0.5 * (std::f32::consts::TAU * i as f32 / 48.0).sin())
            .collect();
        let level = Level::of(&sine);
        assert!(close(level.peak, 0.5));
        assert!(close(level.rms, 0.5 / 2.0_f32.sqrt()));
    }

    #[test]
    fn level_of_a_square_wave_and_silence() {
        let level = Level::of(&[0.25, -0.25, 0.25, -0.25]);
        assert!(close(level.rms, 0.25));
        assert!(close(level.peak, 0.25));
        assert_eq!(Level::of(&[]), Level::default());
        assert_eq!(
            Level::of(&[0.0; 8]),
            Level {
                rms: 0.0,
                peak: 0.0
            }
        );
    }
}
