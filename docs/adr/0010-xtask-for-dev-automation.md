# 0010. cargo xtask for dev automation

- **Status:** Accepted, done in #41
- **Date:** 2026-10-07

## Context

The prototype used bash scripts and a compiled helper (`cargo_tool.rs`) that
didn't work on Windows, where most of the work happens.

## Decision

Dev commands live in an `xtask` crate: `cargo xtask ci` runs the same checks
as CI, and `cargo xtask fmt` formats everything.

## Consequences

- Same behaviour on Windows, macOS and Linux, written in Rust.
- A new command means Rust code instead of a one-line script.

## Reconsider if

Nothing planned.

Alternatives considered: shell scripts, `just`.
