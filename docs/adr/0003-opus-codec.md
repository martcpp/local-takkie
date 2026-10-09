# 0003. Opus for voice

- **Status:** Accepted; binding `opus` 0.4, chosen in #54 (E4.3)
- **Date:** 2026-10-07, updated 2026-10-08

## Context

Voice has to cross Wi-Fi with some packet loss, at low delay, on desktop and
phones.

## Decision

Encode voice with Opus: mono, 48 kHz, 20 ms frames, about 24 kbit/s, with
in-band FEC and packet loss concealment, through the `opus` crate 0.4, which
builds libopus 1.6.1 with `opusic-sys`.

## Consequences

- About 5 KB/s per talker including headers, and lost packets are hidden well.
- libopus is C, built with CMake, on every platform including Android (with
  the NDK, `ANDROID_NDK_HOME` and Ninja).
- `opus` 0.3 used the unmaintained `audiopus_sys`, which also needed two
  Windows fixes (#35). Moving to 0.4 removed all three (#162).

## Reconsider if

A binding that doesn't need CMake becomes solid. In #54, `opus-rs` (pure
Rust) panicked with FEC on, couldn't decode FEC and overshot the bitrate, but
it's worth checking again.

Alternatives considered: raw PCM (30 to 60 times the bandwidth), other codecs.
