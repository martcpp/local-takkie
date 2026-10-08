# 0007. rtrb ring buffers next to the audio callbacks

- **Status:** Accepted
- **Date:** 2026-10-07

## Context

Audio callbacks run on real-time threads. Locking, allocating or logging
there causes clicks and dropouts. The prototype locks mutexes and allocates
in its callbacks.

## Decision

Callbacks only copy samples to or from `rtrb` single-producer,
single-consumer ring buffers. All other work happens on normal threads.

## Consequences

- Wait-free and allocation-free in the callback.
- Overflow and underrun must be counted and handled explicitly.

## Reconsider if

Nothing planned.

Alternatives considered: `ringbuf`, a mutex around a `VecDeque` (the
prototype).
