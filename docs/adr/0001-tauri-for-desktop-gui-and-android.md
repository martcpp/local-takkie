# 0001. Tauri 2 for the desktop GUI and the Android app

- **Status:** Accepted, confirmed by the Android spike ([0011](0011-android-feasibility.md))
- **Date:** 2026-10-07, updated 2026-10-08

## Context

We need a desktop GUI (Windows, macOS, Linux) and an Android app, built and
maintained by one developer. Both must use the same Rust engine.

## Decision

Use Tauri 2 for both. The UI is a web frontend (Svelte + TypeScript) and the
engine runs in-process on the Rust side. Android-only needs (multicast lock,
foreground service, permissions, audio routing) go in a small Kotlin plugin.

## Consequences

- One UI codebase for desktop and mobile, and no FFI layer for the engine.
- Linux GUI builds need WebKitGTK, but the terminal app stays free of it.
- Background audio on Android depends on our own Kotlin plugin.
- Apps are bigger than fully native ones.

## Reconsider if

The Android spike (E4) shows that background audio or audio latency can't be
made to work in Tauri. Then: a native Kotlin UI with UniFFI bindings to
`takkie-engine`.

Alternatives considered: Slint, Dioxus, egui, native Kotlin + UniFFI.
