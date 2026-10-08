# 0003. Opus for voice

- **Status:** Accepted; the Rust binding is chosen in #54 (E4.3)
- **Date:** 2026-10-07, updated 2026-10-08

## Context

Voice has to cross Wi-Fi with some packet loss, at low delay, on desktop and
phones.

## Decision

Encode voice with Opus: mono, 48 kHz, 20 ms frames, about 24 kbit/s, with
in-band FEC and packet loss concealment. For now through the `opus` crate
(libopus).

## Consequences

- About 5 KB/s per talker including headers, and lost packets are hidden well.
- libopus is C, built with CMake. On Windows that needed two fixes (#35), and
  `audiopus_sys`, which `opus` 0.3 uses, is now unmaintained (an exception in
  `deny.toml` until #54).

## Reconsider if

libopus won't cross-compile for Android, or #54 finds a better binding (ideally
one that doesn't need CMake).

Alternatives considered: raw PCM (30 to 60 times the bandwidth), other codecs.
