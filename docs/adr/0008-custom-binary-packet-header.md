# 0008. A small custom binary packet header

- **Status:** Accepted
- **Date:** 2026-10-07

## Context

Every UDP packet needs a version, sender, sequence number, channel and flags.
The desktop and mobile apps, and maybe Kotlin or Swift code later, must agree
on it.

## Decision

A fixed 24-byte big-endian header, written and parsed by hand in
`takkie-core`, specified in `docs/protocol.md`.

## Consequences

- Zero-copy, easy to version, fuzz and implement in any language.
- We own the parser, so it gets property tests and a fuzz target (E5.3).

## Reconsider if

The protocol grows complex enough that a schema language pays off.

Alternatives considered: bincode, protobuf.
