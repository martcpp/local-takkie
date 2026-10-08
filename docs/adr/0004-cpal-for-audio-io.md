# 0004. cpal for audio input and output

- **Status:** Accepted
- **Date:** 2026-10-07

## Context

We need microphone and speaker access on Windows (WASAPI), macOS (CoreAudio),
Linux (ALSA, PulseAudio, PipeWire) and Android (AAudio).

## Decision

Use `cpal` for all of them, behind our own `AudioSource` and `AudioSink`
traits so tests can use fake devices.

## Consequences

- One API across every platform we target.
- Devices differ in rate, sample format and channel count, so the engine must
  negotiate and convert (E6.2, E6.3).
- On Android, cpal needs API level 26 or newer.

## Reconsider if

Audio quality or latency on Android isn't good enough. Then: Oboe.

Alternatives considered: platform-specific code per OS.
