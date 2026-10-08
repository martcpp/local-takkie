# 0009. Half-duplex by default

- **Status:** Accepted
- **Date:** 2026-10-07

## Context

People will use laptop and phone speakers. We have no echo cancellation.

## Decision

While you transmit, your own playback is muted, like a real walkie-talkie.
It can be switched off in settings.

## Consequences

- No echo loops on speakers.
- You can't hear someone who talks over you while you transmit.

## Reconsider if

Users want phone-call style conversation. Then: full duplex with echo
cancellation (for example WebRTC's audio processing, a C++ build).

Alternatives considered: full duplex plus echo cancellation.
