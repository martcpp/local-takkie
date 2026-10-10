//! The hardware boundary: the pipeline only sees these traits, so tests can
//! run it with fakes and no sound card.

/// Mono samples from a microphone.
pub trait AudioSource: Send {
    /// Samples per second.
    fn sample_rate(&self) -> u32;

    /// Copies waiting samples into `out` and returns how many. Never blocks.
    fn read(&mut self, out: &mut [f32]) -> usize;
}

/// Mono samples for a speaker.
pub trait AudioSink: Send {
    /// Samples per second.
    fn sample_rate(&self) -> u32;

    /// Room left, in samples.
    fn free(&self) -> usize;

    /// Samples waiting to play.
    fn queued(&self) -> usize;

    /// Queues as many samples as fit and returns how many. Never blocks.
    fn write(&mut self, samples: &[f32]) -> usize;
}
