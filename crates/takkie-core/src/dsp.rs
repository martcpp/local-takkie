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

/// A second-order filter that keeps its state between frames.
#[derive(Clone, Debug)]
pub struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}

impl Biquad {
    /// Removes what lies below `cutoff` Hz, 12 dB per octave, and leaves
    /// the rest flat.
    #[must_use]
    pub fn high_pass(cutoff: f32, sample_rate: u32) -> Self {
        let omega = std::f64::consts::TAU * f64::from(cutoff) / f64::from(sample_rate);
        let (sin, cos) = omega.sin_cos();
        let alpha = sin / std::f64::consts::SQRT_2;
        let a0 = 1.0 + alpha;
        Self {
            b: [
                (1.0 + cos) / 2.0 / a0,
                -(1.0 + cos) / a0,
                (1.0 + cos) / 2.0 / a0,
            ],
            a: [-2.0 * cos / a0, (1.0 - alpha) / a0],
            z: [0.0; 2],
        }
    }

    /// Lifts (or, with a negative `gain_db`, dips) a band around `centre`
    /// Hz. A smaller `q` makes the band wider.
    #[must_use]
    pub fn peaking(centre: f32, gain_db: f32, q: f32, sample_rate: u32) -> Self {
        let amount = 10_f64.powf(f64::from(gain_db) / 40.0);
        let omega = std::f64::consts::TAU * f64::from(centre) / f64::from(sample_rate);
        let (sin, cos) = omega.sin_cos();
        let alpha = sin / (2.0 * f64::from(q));
        let a0 = 1.0 + alpha / amount;
        Self {
            b: [
                (1.0 + alpha * amount) / a0,
                -2.0 * cos / a0,
                (1.0 - alpha * amount) / a0,
            ],
            a: [-2.0 * cos / a0, (1.0 - alpha / amount) / a0],
            z: [0.0; 2],
        }
    }

    /// Turns everything below about `corner` Hz up or down by `gain_db`.
    #[must_use]
    pub fn low_shelf(corner: f32, gain_db: f32, sample_rate: u32) -> Self {
        let amount = 10_f64.powf(f64::from(gain_db) / 40.0);
        let omega = std::f64::consts::TAU * f64::from(corner) / f64::from(sample_rate);
        let (sin, cos) = omega.sin_cos();
        let slope = 2.0 * amount.sqrt() * sin / std::f64::consts::SQRT_2;
        let (more, less) = (amount + 1.0, amount - 1.0);
        let a0 = more + less * cos + slope;
        Self {
            b: [
                amount * (more - less * cos + slope) / a0,
                2.0 * amount * (less - more * cos) / a0,
                amount * (more - less * cos - slope) / a0,
            ],
            a: [
                -2.0 * (less + more * cos) / a0,
                (more + less * cos - slope) / a0,
            ],
            z: [0.0; 2],
        }
    }

    /// Filters `samples` in place.
    pub fn process(&mut self, samples: &mut [f32]) {
        let [b0, b1, b2] = self.b;
        let [a1, a2] = self.a;
        let [mut z1, mut z2] = self.z;
        for sample in samples {
            let x = f64::from(*sample);
            let y = b0 * x + z1;
            z1 = b1 * x - a1 * y + z2;
            z2 = b2 * x - a2 * y;
            *sample = y as f32;
        }
        // A tail that decays for ever turns into denormals, which are slow.
        self.z = [z1, z2].map(|z| if z.abs() < 1e-20 { 0.0 } else { z });
    }

    /// Forgets what came before.
    pub fn reset(&mut self) {
        self.z = [0.0; 2];
    }
}

const FADE_MS: usize = 5;

/// A short sine tone with faded edges, made a buffer at a time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tone {
    step: f32,
    phase: f32,
    at: usize,
    len: usize,
    fade: usize,
}

impl Tone {
    /// A tone of `frequency` hertz lasting `millis` at `sample_rate`.
    #[must_use]
    pub fn new(frequency: f32, millis: usize, sample_rate: u32) -> Self {
        let rate = sample_rate.max(1) as usize;
        let len = rate * millis / 1_000;
        Self {
            step: std::f32::consts::TAU * frequency / rate as f32,
            phase: 0.0,
            at: 0,
            len,
            fade: (rate * FADE_MS / 1_000).clamp(1, (len / 2).max(1)),
        }
    }

    /// Adds the next samples into `out` with peak level `gain`. A gain of 0
    /// moves the tone on without being heard.
    pub fn mix_into(&mut self, out: &mut [f32], gain: f32) {
        let left = self.len.saturating_sub(self.at);
        for sample in out.iter_mut().take(left) {
            let from_edge = self.at.min(self.len - 1 - self.at);
            let edge = (from_edge as f32 / self.fade as f32).min(1.0);
            *sample += gain * edge * self.phase.sin();
            self.phase = (self.phase + self.step) % std::f32::consts::TAU;
            self.at += 1;
        }
    }

    /// Whether all of it has been made.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.at >= self.len
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
    fn a_tone_has_its_pitch_length_and_soft_edges() {
        let mut tone = Tone::new(1_000.0, 100, 48_000);
        let mut out = vec![0.0_f32; 6_000];
        tone.mix_into(&mut out[..1_000], 0.5);
        assert!(!tone.is_done());
        tone.mix_into(&mut out[1_000..], 0.5);
        assert!(tone.is_done());

        let (sound, after) = out.split_at(4_800);
        assert!(after.iter().all(|sample| *sample == 0.0));
        let crossings = sound
            .windows(2)
            .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
            .count();
        assert!((198..=202).contains(&crossings), "{crossings} crossings");
        let peak = Level::of(sound).peak;
        assert!((peak - 0.5).abs() < 0.01, "peak {peak}");
        assert!(Level::of(&sound[..24]).peak < 0.06);
        assert!(Level::of(&sound[4_776..]).peak < 0.06);
    }

    #[test]
    fn a_tone_adds_to_what_is_there_and_can_pass_unheard() {
        let mut out = [0.25_f32; 480];
        let mut silent = Tone::new(1_000.0, 10, 48_000);
        silent.mix_into(&mut out, 0.0);
        assert!(silent.is_done());
        assert_eq!(out, [0.25; 480]);

        let mut heard = Tone::new(1_000.0, 10, 48_000);
        heard.mix_into(&mut out, 0.5);
        assert!(out.iter().any(|sample| (*sample - 0.25).abs() > 0.2));
        heard.mix_into(&mut out, 0.5);

        let mut empty = Tone::new(1_000.0, 0, 48_000);
        assert!(empty.is_done());
        empty.mix_into(&mut out, 0.5);
    }

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

    fn sine(frequency: f32, amplitude: f32) -> Vec<f32> {
        (0..48_000)
            .map(|i| amplitude * (std::f32::consts::TAU * frequency * i as f32 / 48_000.0).sin())
            .collect()
    }

    fn settled(filter: &mut Biquad, mut samples: Vec<f32>) -> f32 {
        for frame in samples.chunks_mut(960) {
            filter.process(frame);
        }
        Level::of(&samples[24_000..]).rms
    }

    #[test]
    fn a_high_pass_cuts_rumble_and_leaves_the_voice() {
        let rms = 0.5 / 2.0_f32.sqrt();
        let mut filter = Biquad::high_pass(100.0, 48_000);
        assert!(settled(&mut filter, sine(25.0, 0.5)) < rms * 0.08);
        filter.reset();
        assert!(settled(&mut filter, sine(50.0, 0.5)) < rms * 0.3);
        filter.reset();
        let at_cutoff = settled(&mut filter, sine(100.0, 0.5));
        assert!((at_cutoff / rms - 0.707).abs() < 0.02, "{at_cutoff}");
        for voice in [300.0, 1_000.0, 4_000.0, 12_000.0] {
            filter.reset();
            let out = settled(&mut filter, sine(voice, 0.5));
            assert!((out / rms - 1.0).abs() < 0.02, "{voice} Hz: {out}");
        }
    }

    fn gain_db(filter: &mut Biquad, frequency: f32) -> f32 {
        filter.reset();
        let out = settled(filter, sine(frequency, 0.1));
        20.0 * (out / (0.1 / 2.0_f32.sqrt())).log10()
    }

    #[test]
    fn a_peaking_filter_lifts_its_band_and_leaves_the_rest() {
        let mut filter = Biquad::peaking(3_000.0, 6.0, 0.8, 48_000);
        assert!((gain_db(&mut filter, 3_000.0) - 6.0).abs() < 0.1);
        assert!(gain_db(&mut filter, 200.0).abs() < 0.2);
        assert!(gain_db(&mut filter, 18_000.0).abs() < 0.8);
        let edge = gain_db(&mut filter, 1_500.0);
        assert!((1.0..4.0).contains(&edge), "{edge}");
    }

    #[test]
    fn a_low_shelf_turns_the_lows_down_and_leaves_the_rest() {
        let mut filter = Biquad::low_shelf(250.0, -4.0, 48_000);
        assert!((gain_db(&mut filter, 60.0) + 4.0).abs() < 0.3);
        assert!((gain_db(&mut filter, 250.0) + 2.0).abs() < 0.3);
        assert!(gain_db(&mut filter, 2_000.0).abs() < 0.1);
    }

    #[test]
    fn a_filter_with_no_gain_changes_nothing_audible() {
        let mut filter = Biquad::peaking(3_000.0, 0.0, 0.8, 48_000);
        assert!(gain_db(&mut filter, 3_000.0).abs() < 0.001);
    }

    #[test]
    fn a_high_pass_removes_a_constant_offset() {
        let mut filter = Biquad::high_pass(100.0, 48_000);
        assert!(settled(&mut filter, vec![0.3; 48_000]) < 1e-6);
    }

    #[test]
    fn a_filter_is_the_same_in_one_piece_or_in_frames() {
        let input = sine(180.0, 0.4);
        let mut whole = input.clone();
        Biquad::high_pass(100.0, 48_000).process(&mut whole);
        let mut framed = input;
        let mut filter = Biquad::high_pass(100.0, 48_000);
        for frame in framed.chunks_mut(960) {
            filter.process(frame);
        }
        assert_eq!(whole, framed);
    }

    #[test]
    fn a_filter_stays_finite_and_goes_quiet_after_the_sound_stops() {
        let mut filter = Biquad::high_pass(100.0, 48_000);
        let mut burst = sine(200.0, 1.0);
        filter.process(&mut burst);
        let mut silence = vec![0.0; 480_000];
        filter.process(&mut silence);
        assert!(silence.iter().all(|sample| sample.is_finite()));
        assert_eq!(silence[479_000..], [0.0; 1_000]);
    }
}
