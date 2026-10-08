# 0002. Plain threads and channels in the engine, no async runtime

- **Status:** Accepted
- **Date:** 2026-10-07

## Context

The engine moves audio between real-time device callbacks, a codec and a UDP
socket at about 50 packets a second per sender. Audio callbacks can't be
async.

## Decision

The engine uses named threads, `crossbeam-channel` messages, atomics for hot
flags and lock-free ring buffers. No tokio or other async runtime.

## Consequences

- Easy to reason about and to test with fake devices.
- Every thread needs a clean stop path; the engine's `Drop` owns that.
- The Tauri app runs tokio for its own needs, which is fine: it only calls
  the engine's plain methods.

## Reconsider if

We add internet relays or many sockets per engine.

Alternatives considered: tokio everywhere.
