# 0012. RNNoise (`nnnoiseless`) for noise suppression

- **Status:** Accepted
- **Date:** 2026-10-12

## Context

Takkie is meant to work in loud rooms: a church with a PA and a crowd is the
first real test. Until now the microphone went straight into Opus, noise and
all. We want the noise pushed down before encoding, on a budget phone, with
little added delay and no C or C++ build.

Two candidates fit, both pure Rust and BSD-3-Clause:

- `sonora` 0.2: WebRTC's noise suppressor (a classic spectral method).
- `nnnoiseless` 0.5: RNNoise, a small neural network.

Both were run on the same made-up clips: synthesised speech, a reverberant
crowd of nine voices or pink noise at several levels, and a low cut first, as
in the app. "Intelligibility" is a simplified STOI (how well the talker's
loudness pattern survives in each band; 1.0 is perfect). "Noise" is the level
in the talker's pauses.

| Clip | Processor | Intelligibility | Noise |
|---|---|---|---|
| Crowd, 10 dB quieter than the voice | none | 0.921 | -33.0 dB |
| | sonora, moderate | 0.916 | -44.2 dB |
| | sonora, very high | 0.891 | -50.8 dB |
| | RNNoise | 0.952 | -50.9 dB |
| | RNNoise, 85% blend | 0.951 | -46.4 dB |
| Crowd as loud as the voice | none | 0.791 | -23.0 dB |
| | sonora, moderate | 0.789 | -33.5 dB |
| | sonora, very high | 0.756 | -39.4 dB |
| | RNNoise | 0.834 | -38.8 dB |
| | RNNoise, 85% blend | 0.839 | -35.2 dB |
| Hiss, 10 dB quieter than the voice | none | 0.917 | -33.1 dB |
| | sonora, moderate | 0.921 | -45.3 dB |
| | RNNoise | 0.953 | -56.4 dB |
| Talker far from the microphone, crowd | none | 0.718 | -26.7 dB |
| | sonora, moderate | 0.707 | -32.9 dB |
| | RNNoise | 0.714 | -31.9 dB |
| | RNNoise, 70% blend | 0.739 | -30.6 dB |

RNNoise removed as much noise as sonora's hardest setting and raised the
intelligibility score, where sonora left it flat or lowered it. It also kept
the voice at its level; sonora lowered it by 2 to 5 dB when the noise was
loud.

## Decision

Use `nnnoiseless` as a stage of the voice chain, after the low cut. Three
settings: off, normal (85% cleaned, 15% microphone) and strong (100%). Normal
is the default: it scored as clear as strong and leaves a little of the room,
which sounds less processed.

It only runs on frames that are sent, so an idle radio pays nothing.

## Consequences

- 10 ms more delay on the sending side.
- More CPU while talking: roughly 0.15 to 0.3 ms per 10 ms on a laptop.
- No level between "most" and "all": the blend is our own, and it only works
  because RNNoise's delay is exactly one block.
- `nnnoiseless` uses `unsafe` in two places for speed. Our own crates still
  forbid it.
- Nothing helps a talker who is far from the microphone: every processor
  scored about the same there. That stays a matter of holding the microphone
  close.

## Reconsider if

- Listening in a real church disagrees with these numbers. The test speech
  was synthetic, and real crowd noise is harder than nine synthetic voices.
- We add echo cancellation and automatic gain from `sonora`: one library for
  everything may then be worth a little quality.
- A stronger model (DeepFilterNet) proves fast enough on a phone.

Alternatives considered: `sonora`'s noise suppressor; the phone's own
processing (Android only, and it differs from phone to phone); DeepFilterNet
(40 ms of delay and several times the CPU).
