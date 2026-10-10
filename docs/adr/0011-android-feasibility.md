# 0011. Android is feasible with Tauri, cpal, Opus and mdns-sd

- **Status:** Accepted
- **Date:** 2026-10-08

## Context

ADR 0001 chose Tauri 2 for the Android app on the condition that a spike
(E4, #52 to #56) shows background audio and audio latency work. The spike
also had to check that our audio, codec and discovery crates run on a phone.

Everything was tested on one phone, a REDMI 15C (Android 16, HyperOS 3.0,
arm64), with a Windows laptop on the same Wi-Fi. The code is on the
`spike/android` branch, and each ticket has the full results.

## Decision

**Go.** Build the Android app as planned: Tauri 2 for the UI, the Rust engine
in-process, cpal for audio, `opus` 0.4 for the codec, mdns-sd for discovery,
and a small Kotlin plugin for the Android-only parts. The minimum Android
version is 8.0 (API 26), which AAudio needs.

## What we measured

| Area | Result |
|---|---|
| Toolchain (#52) | Command-line Android SDK and NDK, no Android Studio. A debug APK installs and runs over USB. |
| Audio (#53) | cpal mic to speaker works through AAudio. Round trip through the speaker about **100 ms**, judged fine for push-to-talk. |
| Codec (#54) | `opus` 0.4 cross-compiles with the NDK and runs on the phone: about 0.5 ms to encode a 20 ms frame. All libopus bindings decode each other's packets. |
| Discovery (#55) | mdns-sd finds peers both ways, phone to laptop, on a router and on the phone's own hotspot, with a Wi-Fi `MulticastLock` held. |
| Background (#56) | With a microphone foreground service, mic and speaker ran 10 minutes with the screen off, and the multicast lock stayed active. Battery about 1% an hour. |

## Consequences

- **A foreground service is required, not optional.** Without it, Android
  mutes the mic about 6 s after the screen goes off, stops playback after
  about 70 s, and suspends the multicast lock, all without an error. The
  service has to start while the app is on screen.
- **The Kotlin plugin is small.** It covers the multicast lock, the
  foreground service and the runtime permissions (`RECORD_AUDIO`,
  `POST_NOTIFICATIONS`). Each Kotlin class needs its own Tauri plugin name,
  because Android keeps one plugin per name. The Rust command that calls into
  Kotlin must be `async`, because Kotlin answers on the main thread.
- **Latency is set by the phone, not by us.** cpal never asks AAudio for
  low-latency mode, and on Xiaomi phones the MiSound effect blocks the fast
  speaker path for every app anyway. Asking for low latency costs nothing
  where it's refused, so we add it to cpal (new E6 ticket) instead of writing
  our own AAudio backend, which would need `unsafe`.
- **Ask for 48 kHz mono explicitly.** On Android, cpal's default config is
  44.1 kHz stereo and its device list is generic, not real.
- **Announce only LAN addresses.** On a phone, mdns-sd's automatic addresses
  include loopback, mobile data and virtual interfaces, which peers can't
  reach (E7.2). Recognise yourself by an ID in the TXT record, not by name:
  a quick restart renamed the phone `phone (2)` and it then saw itself.
- **Expect looser timing in the background.** Speaker underruns rose to about
  four a second with the screen off, so the jitter buffer needs some slack.
- **Building on Windows needs a little setup.** Developer Mode (Tauri links
  with a symlink), `ANDROID_NDK_HOME`, and Ninja for CMake (the SDK's `cmake`
  package has it). Xiaomi phones also need "Install via USB" turned on.

## Open questions

- **One phone is not enough.** Low-latency mode and voice mode (echo
  cancellation) should be tried on at least one non-Xiaomi phone before M4.
- **Voice mode** (tagging streams as a voice call) skips MiSound and turns on
  echo cancellation, but on the Redmi it was slower. Decide once a second
  phone has been tested. It would raise the minimum to Android 9 (API 28).

## Reconsider if

Another phone shows background audio or latency that cpal can't fix, or a
future Android release stops foreground services of type `microphone` from
keeping the mic open. Then: native Kotlin audio (Oboe) behind the engine's
audio traits, or the native Kotlin UI with UniFFI from ADR 0001.
