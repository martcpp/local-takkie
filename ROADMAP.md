# local-takkie — Roadmap

> Status: **draft for review** · Last updated: 2026-10-07
>
> Planning basis: **one developer, about 25 focused hours a week** (3–4 hours on
> weekdays, more at weekends). See [section 8](#8-order-of-work-and-schedule) for
> the schedule.
>
> This document is the plan for taking local-takkie from a prototype to a
> released, cross-platform product. Read it, change it, and approve it. After
> that, each **epic** (E1, E2, …) becomes an epic in the ticket tracker, and
> each **work item** (E5.2, E5.3, …) becomes a ticket under it.

---

## Contents

1. [Product vision and scope](#1-product-vision-and-scope)
2. [Where the project is today](#2-where-the-project-is-today)
3. [Target architecture](#3-target-architecture)
4. [Key technical decisions](#4-key-technical-decisions)
5. [Rust engineering standards](#5-rust-engineering-standards)
6. [Branching, CI and releases](#6-branching-ci-and-releases)
7. [Milestones and epics](#7-milestones-and-epics)
8. [Order of work and schedule](#8-order-of-work-and-schedule)
9. [Risks and how we reduce them](#9-risks-and-how-we-reduce-them)
10. [Decisions and assumptions](#10-decisions-and-assumptions)
11. [Glossary](#11-glossary)

---

## 1. Product vision and scope

**local-takkie is a walkie-talkie for the local network.** People on the same
Wi-Fi or LAN find each other automatically, pick one of **10 channels**, and
talk by holding a push-to-talk (PTT) button. No internet, no server, no
account.

### Apps

| App | Platforms | UI | Milestone |
|---|---|---|---|
| `takkie` (terminal app) | Windows, macOS, Linux | Terminal UI (ratatui) | M1 |
| Takkie Desktop | Windows, macOS, Linux | GUI (Tauri 2) | M3 |
| Takkie for Android | Android 8.0+ | GUI (Tauri 2, same UI code as desktop) | M4 |
| Takkie for iOS | iOS | GUI (Tauri 2) | After v1.0 (see [decisions](#10-decisions-and-assumptions)) |

All apps share **one Rust engine** and speak **one wire protocol**, so any app
can talk to any other app on the same channel.

### Features in scope for v1.0

- Automatic discovery of other users on the LAN (mDNS), plus manual peer entry
  for networks that block discovery.
- Push-to-talk: hold to talk; toggle mode where hold is not possible.
- **10 channels** (1–10). You only hear people on your channel.
- **Optional passphrase per channel.** Audio on a channel with a passphrase is
  encrypted. People without the passphrase cannot listen or inject audio.
- Half-duplex by default, like a real walkie-talkie: your speaker is muted
  while you talk.
- "Who is talking" indicator, real microphone level meter, mute and volume.
- Choice of microphone and speaker device.
- Several people can talk at the same time without the audio breaking (it is
  mixed).
- Installers and downloadable builds for every supported platform.

### Out of scope for v1.0 (possible later)

iOS app, noise suppression, recording, text chat, Bluetooth PTT accessories,
IPv6-only networks, talking across the internet.

---

## 2. Where the project is today

### What exists

A single binary (`vl <name> <port>`, crate `videolan`) with a terminal UI.

- **Discovery:** it announces itself with mDNS and finds peers.
- **Talking:** while SPACE is held, it records the mic at 48 kHz, compresses
  it with Opus and sends it over UDP to every peer.
- **Listening:** it decodes incoming audio and plays it.

### Branches

| Branch | State |
|---|---|
| `main` | Last commit 2026-02-19 |
| `develop` | 2 commits ahead of `main` (2026-03-02): library split, about 40 tests, 3-OS test workflow, pre-commit scripts. **Not merged.** |
| PR #3 | Still open, superseded by later work |
| Other branches | 7 stale branches |

**CI has failed on every run.**

### Baseline check on Windows (2026-10-07)

Checked on `main`, Windows 11, Rust 1.98.1. macOS and Linux were not tested.
Audio between two machines was not tested (that needs two devices).

| Check | Result |
|---|---|
| Build without CMake | ❌ Fails. libopus is compiled from C source and needs CMake. |
| Build with CMake 4.x | ❌ Fails. The bundled libopus 1.3 asks for CMake 3.1 compatibility, which CMake 4 removed. **Fix:** `CMAKE_POLICY_VERSION_MINIMUM=3.5`. |
| Debug build, link step | ❌ Fails with `unresolved external symbol __imp__CrtDbgReportW`. libopus is built with the debug C runtime, Rust uses the release one. **Fix (verified):** `[profile.dev.package.audiopus_sys] opt-level = 1`. |
| Visual Studio generator | On this PC CMake picked "Visual Studio 18 2026" and failed. `CMAKE_GENERATOR="Visual Studio 17 2022"` worked. Probably specific to this machine. |
| `cargo build` with the fixes above | ✅ Builds `vl.exe` |
| `cargo clippy --all-targets -- -D warnings` | ❌ 2 errors: `collapsible_if` (`src/ui/tui.rs:111`), `manual_is_multiple_of` (`src/audio/rad.rs:96`) |
| `cargo fmt --check` | ❌ 7 places in `rad.rs`, `sad.rs`, `tui.rs` |
| `cargo test` | ✅ Passes, but **0 tests exist on `main`** (`develop` has about 40) |
| Future-incompatibility warning | `net2 v0.2.39`, pulled in by the unused `mdns 3.0` dependency. It will stop compiling in a future Rust version. Removed by E1.3. |

### Main problems found in the code

| # | Problem | Effect |
|---|---|---|
| 1 | Sample rate forced to 48 kHz and format to f32, then `.unwrap()` | Crash on devices that don't support it (many Bluetooth headsets, some Mac/Linux mics) |
| 2 | Devices with more than 2 channels are treated as stereo | Garbled audio on laptop mic arrays and 5.1 outputs |
| 3 | One shared Opus decoder for all senders | Two people talking at once produces garbage |
| 4 | No packet header (no sender, sequence number or version) | No handling of lost or reordered packets, no "who is talking", any stray UDP packet is played as audio |
| 5 | Locks, memory allocation, logging and network sends inside the real-time audio callbacks | Crackles and dropouts |
| 6 | Buffers have no size limit and there is no jitter buffer | Delay grows over time, and memory grows if playback stalls |
| 7 | PTT release timeout (200 ms) is shorter than keyboard auto-repeat delay (250–600 ms) on macOS/Linux | Transmission flickers on and off |
| 8 | mDNS: peers that leave are never removed. The address may be IPv6 while the socket is IPv4-only, and VPN/WSL adapters can be picked. Self-detection is fragile. | Ghost peers, unreachable peers |
| 9 | Logger disabled because of the TUI. Panics in background threads are silent. | Errors are invisible |
| 10 | No clean shutdown, and the terminal is not restored after a panic | Broken terminal after a crash |
| 11 | TUI shows fake values ("Audio Level" is 100% when PTT is on, and "samples" is really bytes) | Misleading |
| 12 | 22 of 30 dependencies are unused. `tauri-plugin-os` pulls Tauri, GTK and WebKit into Linux builds. | 698 crates in the lock file, slow builds, extra Linux system packages |
| 13 | Repo clutter: a committed `.exe`, an empty `discovery.rs`, a `setup.sh` that runs the app, and outdated `AUDIO_STREAMING.md` | Confusing |
| 14 | Named `videolan` / "VideoLAN Audio Streamer" (the VLC organisation's name) | Name clash |

**Conclusion:** keep the idea and the libraries (cpal, Opus, mdns-sd,
ratatui), and **rebuild the engine** with a proper protocol and a real-time-safe
audio pipeline. Patching the current code would cost more than rebuilding it.

---

## 3. Target architecture

### 3.1 Cargo workspace layout

```text
local-takkie/
├── Cargo.toml                  # [workspace]: shared deps, lints, profiles
├── crates/
│   ├── takkie-core/            # Pure logic. No I/O, no C code, no threads.
│   │                           #   protocol (packet format), crypto, ChannelId,
│   │                           #   jitter buffer, DSP helpers (mix, level),
│   │                           #   PTT state machine, peer table
│   ├── takkie-engine/          # The running engine. All I/O lives here.
│   │                           #   audio devices (cpal), Opus, resampling,
│   │                           #   UDP transport, mDNS discovery, threads.
│   │                           #   Public API: Engine, EngineEvent, EngineConfig
│   └── takkie-tui/             # `takkie` binary: ratatui terminal app + clap CLI
├── apps/
│   └── takkie-app/             # Tauri 2 app (desktop GUI + Android)
│       ├── src-tauri/          #   Rust side, depends on takkie-engine
│       ├── ui/                 #   Web frontend (TypeScript + Svelte)
│       └── plugins/takkie-mobile/  # Tauri plugin: Rust + Kotlin (Android)
├── xtask/                      # `cargo xtask ci` etc. (replaces cargo_tool.rs + shell scripts)
├── docs/
│   ├── adr/                    # Architecture Decision Records
│   ├── protocol.md             # Wire protocol spec
│   └── security.md             # Threat model for channels and encryption
└── .github/workflows/          # ci.yml, release.yml
```

**Why split this way**

- `takkie-core` has no I/O, so it is fast to test, can be fuzzed, and compiles
  for every target (including Android and iOS) with no system libraries.
- `takkie-engine` is the only crate that touches hardware and the network.
  Every UI uses it through one small API.
- The TUI does not depend on Tauri, so the terminal app stays small and builds
  without GTK/WebKit.
- The Tauri app is outside `crates/` because it has its own frontend toolchain
  (Node) and mobile build. It is still a workspace member on the Rust side.

### 3.2 Engine API (the contract every UI uses)

A sketch. The exact names are decided in E8.

```rust
pub struct EngineConfig {
    pub display_name: String,             // default: hostname
    pub port: u16,                        // 0 = OS picks a free port
    pub channel: ChannelId,               // 1..=10
    pub passphrase: Option<Passphrase>,   // None = open channel
    pub input_device: Option<String>,     // None = system default
    pub output_device: Option<String>,
    pub static_peers: Vec<SocketAddr>,    // for networks that block mDNS
    pub half_duplex: bool,                // default: true
}

/// Owns all engine threads. Dropping it shuts everything down cleanly.
pub struct Engine { /* ... */ }

impl Engine {
    pub fn start(config: EngineConfig)
        -> Result<(Engine, crossbeam_channel::Receiver<EngineEvent>), EngineError>;

    pub fn set_transmitting(&self, on: bool);
    pub fn set_channel(&self, channel: ChannelId, passphrase: Option<Passphrase>);
    pub fn set_muted(&self, muted: bool);
    pub fn set_volume(&self, volume: f32);
    pub fn list_devices() -> Result<DeviceList, EngineError>;

    /// Cheap, lock-free snapshot for drawing the UI (peers, levels, stats).
    pub fn snapshot(&self) -> EngineSnapshot;
}

pub enum EngineEvent {
    PeerJoined(PeerInfo),
    PeerLeft(PeerId),
    PeerUpdated(PeerInfo),              // name or channel changed
    TalkStarted(PeerId),
    TalkStopped(PeerId),
    WrongPassphrase(PeerId),            // same channel, different passphrase
    DeviceLost(DeviceKind),
    Warning(String),
    Fatal(EngineError),
}
```

How each UI uses the API:

- **TUI:** calls these methods directly.
- **Tauri app:** wraps each method in a `#[tauri::command]` and forwards
  `EngineEvent`s to the frontend with `app.emit(...)`.

### 3.3 Threads and data flow

```text
SENDING
 Mic ─► [input callback]  ─rtrb─►  [tx thread] ─────────────────────────────► UDP ─► peers on my channel
         real-time:                 downmix to mono → resample to 48 kHz
         copy samples only          → PTT gate → level meter → Opus encode
                                    → packet header → encrypt (if passphrase)

RECEIVING
 peers ─► UDP ─► [rx thread] ───────────────► per-sender jitter buffers ─► [mix thread] ─rtrb─► [output callback] ─► Speaker
                  parse header → drop other      (reorder, drop late,         Opus decode / PLC / FEC   real-time:
                  channels → decrypt →           detect loss)                 → mix senders → volume    copy samples only
                  drop replays                                                → resample to device rate

CONTROL
 [discovery thread]  mdns-sd events ─► peer table ─► EngineEvent ─► UI
 [control thread]    UI commands (crossbeam channel) ─► engine state
```

Rules:

- **Audio callbacks are real-time:** no locks, no allocation, no logging, no
  system calls. They only copy samples to or from a lock-free single-producer,
  single-consumer (SPSC) ring buffer (`rtrb`).
- Encoding, decoding, encryption and networking all happen on normal threads.
- The peer list is published with `arc-swap`, so readers never block.
- Hot flags (PTT, mute) are atomics. Everything else uses
  `crossbeam-channel` messages.
- **Plain threads, no async runtime** in the engine. UDP at 50 packets per
  second does not need tokio, and real-time audio callbacks cannot be async.

### 3.4 Audio pipeline

| Setting | Value | Why |
|---|---|---|
| Internal format | Mono, f32, 48 kHz | Voice needs only one channel. 48 kHz is Opus's native rate. |
| Frame size | 20 ms (960 samples) | Standard Opus VoIP frame |
| Codec | Opus, `Application::Voip`, ~24 kbit/s | Good voice quality, about 5 KB/s per sender including headers |
| Loss handling | Opus in-band FEC on, expected loss 10%, PLC for missing packets | Smooths over Wi-Fi packet loss |
| Jitter buffer | Target ~60 ms, max 200 ms, per sender | Absorbs network timing jitter without the delay growing |
| Device rate ≠ 48 kHz | Resample with `rubato` | Works with 44.1 kHz, 16 kHz, Bluetooth, etc. |
| Device sample format | Any cpal format (i16, u16, f32, …), converted to f32 | No more "format not supported" crashes |
| Device channels | Downmix input to mono, copy output to all channels | Works with mic arrays and 5.1 outputs |
| Mixing | Sum active senders + soft limiter | Several talkers at once stay clear |

### 3.5 Wire protocol v1

Each UDP datagram is one packet. Multi-byte fields are big-endian. The full spec
goes in `docs/protocol.md` (E5.2).

| Offset | Size | Field | Meaning |
|---|---|---|---|
| 0 | 2 | `magic` | `0x54 0x4B` ("TK"). Anything else is dropped. |
| 2 | 1 | `version` | `1` |
| 3 | 1 | `kind` | `1` = Audio, `2` = Hello (keep-alive + name), `3` = Bye |
| 4 | 1 | `channel` | `1`–`10` |
| 5 | 1 | `flags` | bit 0 = encrypted, bit 1 = end of transmission (last packet of a PTT press) |
| 6 | 2 | `reserved` | `0` |
| 8 | 8 | `sender_id` | Random number chosen at app start |
| 16 | 4 | `seq` | +1 for every packet this sender sends (all kinds) |
| 20 | 4 | `timestamp` | Position in the audio stream, in 48 kHz samples |
| 24 | n | `payload` | Opus packet (Audio), UTF-8 name (Hello), empty (Bye). If encrypted: ciphertext + 16-byte tag. |

A typical audio packet is about 24 + 60 + 16 ≈ 100 bytes, at 50 packets per
second.

### 3.6 Channels and encryption

**Channels**

- Channels are numbered **1–10** and modelled as a validated newtype
  `ChannelId(u8)`.
- Each app announces its channel in its mDNS record.
- Senders only send to peers on the same channel. Receivers also drop packets
  for other channels.

**Passphrase encryption** (optional per channel)

- **Key:** `Argon2id(passphrase, salt = "local-takkie/v1/channel/<n>")`, with
  m = 19 MiB, t = 2, p = 1. It runs once, when you join the channel. The same
  passphrase on different channels gives different keys.
- **Cipher:** ChaCha20-Poly1305 (RustCrypto `chacha20poly1305`).
- **Nonce:** `sender_id (8 bytes) ‖ seq (4 bytes)`. This never repeats, because
  `sender_id` is random per run and `seq` only grows.
- **Authenticated header:** the 24-byte header is passed as associated data,
  so it cannot be changed without detection.
- **Replays:** each receiver keeps a sliding window of recent `seq` numbers per
  sender and drops duplicates.
- **Key handling:** keys are wiped from memory on drop (`zeroize`). Passphrases
  are never logged.

**What this protects**

- People without the passphrase can't hear the audio and can't inject audio
  into the channel.

**What it does not protect** (written up in `docs/security.md`)

- Everyone on the LAN can see display names, channel numbers and who is
  talking when.
- Anyone who has the passphrase can listen.
- Weak passphrases can be guessed offline by someone who records the traffic.
  The UI warns about short passphrases.

### 3.7 Discovery

- **One `mdns-sd` daemon per app**, with its handle kept for the whole run.
  - Service type `_takkie._udp.local.`
  - TXT record: `v=1`, `id=<sender_id>`, `ch=<channel>`, `name=<display name>`
  - `enable_addr_auto()`, so all real network interfaces are announced and
    kept up to date.
- **Events:** handle `ServiceResolved` and `ServiceRemoved`. When choosing a
  peer's address, prefer usable IPv4 addresses. Recognise yourself by `id`,
  not by IP address.
- **Hello packets** every 2 s keep peers alive and carry the display name. A
  peer that is silent for 10 s (no Hello and no mDNS) is removed.
- **Shutdown:** send a Bye packet, then unregister from mDNS.
- **Fallback:** `--peer <ip:port>` / "Add peer manually", for guest Wi-Fi and
  routers that block multicast.

### 3.8 Frontends

**Terminal app (`takkie`)**

- **CLI (clap):** `--name` (default: hostname), `--channel`, `--port`,
  `--input-device`, `--output-device`, `--list-devices`, `--peer`,
  `--ptt-mode`.
- **PTT:**
  - Real hold-to-talk where the terminal reports key releases: Windows, and
    terminals that support the kitty keyboard protocol (for example kitty,
    foot, Ghostty and Alacritty; WezTerm needs a setting turned on).
  - Toggle mode everywhere else (SPACE to start, SPACE to stop).
  - The footer always shows which mode is active.
- **Keys:** `1`–`9`, `0` = channel 1–10 · `P` passphrase · `M` mute ·
  `+`/`-` volume · `D` devices · `?` help · `Q` quit.

**Desktop GUI (Tauri 2)**

- **PTT:** a big hold-to-talk button (mouse or touch), plus a keyboard key
  while the window is focused. Browsers report key-up reliably, so hold-to-talk
  works on every OS.
- **Global hotkey:** optional, via `tauri-plugin-global-shortcut`.
- **Screens:** main (channel selector, PTT, peers, levels) and settings (name,
  devices, passphrases, PTT key).

**Android (Tauri 2 + Kotlin plugin)**

- **Same UI as desktop,** with a mobile layout.
- **Kotlin plugin** (`plugins/takkie-mobile`) handles what Android needs:
  - runtime permissions (`RECORD_AUDIO`, `POST_NOTIFICATIONS`)
  - Wi-Fi `MulticastLock` (Android drops mDNS traffic without it)
  - a foreground service of type `microphone`, so audio keeps working with the
    screen off
  - audio routing (speaker, earpiece, Bluetooth)
- **Minimum SDK 26** (Android 8.0). cpal's Android backend (AAudio) needs it.

---

## 4. Key technical decisions

Each decision becomes a short Architecture Decision Record (ADR) in `docs/adr/`
during E2. "Reconsider if" says what would make us change it.

| # | Decision | Why | Alternatives considered | Reconsider if |
|---|---|---|---|---|
| D1 | **Tauri 2** for desktop GUI and Android | One UI codebase for desktop + mobile. The Rust engine runs in-process. A Kotlin plugin system covers Android-only needs. It was already planned (`tauri-plugin-os` was in Cargo.toml). | Slint, Dioxus, egui, native Kotlin + UniFFI | The Android spike (E4) shows background audio or audio latency can't be made to work in Tauri. Then: native Kotlin UI + UniFFI bindings to `takkie-engine`. |
| D2 | **Threads + channels**, no tokio in the engine | Real-time audio isn't async. Low packet rate. Simpler to reason about and test. | tokio everywhere | We add internet relays or many sockets |
| D3 | **Opus** via the `opus` crate (libopus) | Industry standard for voice, built-in loss concealment and FEC | Raw PCM (needs 30–60x more bandwidth), other codecs | Cross-compiling libopus for Android fails (E4). Then: prebuilt libopus or a different binding. |
| D4 | **cpal** for audio I/O | One API for WASAPI, CoreAudio, ALSA/PulseAudio/PipeWire and AAudio | Platform-specific code | Android audio quality issues → Oboe |
| D5 | **mdns-sd** for discovery | Pure Rust, works on all 3 desktop OSes and Android (with MulticastLock) | libmdns, Bonjour/NSD via platform plugins | iOS: Apple requires a special multicast entitlement, so we may need NWBrowser via Swift |
| D6 | **ChaCha20-Poly1305 + Argon2id** (RustCrypto) | Fast on phones without AES hardware, misuse-resistant, pure Rust | AES-GCM, PAKE-based key exchange | We need per-user keys or forward secrecy |
| D7 | **rtrb** ring buffers between audio callbacks and threads | Wait-free SPSC, no allocation | `ringbuf`, mutex + VecDeque (current) | — |
| D8 | **Custom binary header** instead of bincode/serde | 24 fixed bytes, zero-copy, easy to version and fuzz, language-neutral (Kotlin/Swift could implement it) | bincode, protobuf | The protocol grows complex |
| D9 | **Half-duplex by default** | Walkie-talkie behaviour, and avoids echo when using speakers (we have no echo cancellation) | Full duplex + echo cancellation (WebRTC APM, C++ build) | Users ask for phone-call style |
| D10 | **xtask** for dev automation | Cross-platform (works on Windows), plain Rust, no bash needed | Shell scripts (current), `just` | — |

---

## 5. Rust engineering standards

These rules apply to every crate and every PR. CI enforces as many of them as
possible.

### 5.1 Toolchain and project setup

- **Edition 2024.** Set `rust-version = "1.88"` (the code uses let-chains) in
  `[workspace.package]`. CI tests this minimum version (MSRV).
- **Dependencies:** a dependency used by two or more crates is declared once
  in `[workspace.dependencies]` and used with `{ workspace = true }`. One used
  by a single crate stays in that crate's `Cargo.toml`.
- **Lints:** shared in `[workspace.lints]`, and each crate opts in with
  `lints.workspace = true`:

  ```toml
  [workspace.lints.rust]
  unsafe_code = "forbid"            # our code needs no unsafe; C libs are wrapped by their crates
  missing_docs = "warn"             # on library crates

  [workspace.lints.clippy]
  all = { level = "warn", priority = -1 }
  unwrap_used = "warn"
  expect_used = "warn"
  panic = "warn"
  todo = "warn"
  dbg_macro = "warn"
  print_stdout = "warn"             # libraries log via tracing, never println!
  ```

  CI runs `cargo clippy --all-targets -- -D warnings`, so every warning above
  fails the build. Tests may `#[allow]` unwrap/expect.
- **Formatting:** `rustfmt.toml` checked in, and CI runs
  `cargo fmt --all --check`.
- **Release profile:** `lto = "thin"`, `codegen-units = 1`, `strip = true`.
- **`Cargo.lock` is committed** (this is an application).

### 5.2 Design rules

- **Make invalid states impossible.** Use newtypes with validation:
  `ChannelId` (1–10, `TryFrom<u8>`), `PeerId`, `Seq`, `Passphrase` (zeroized).
  Use enums instead of booleans plus strings.
- **Pure core, thin I/O.** Logic goes in `takkie-core` as plain functions and
  structs. `takkie-engine` only wires them to devices, sockets and threads.
- **Traits at the hardware boundary:** `AudioSource`, `AudioSink` and
  `Transport`. Tests use fake implementations, so no sound card or network is
  needed.
- **Ownership over sharing.** Give state to one thread and send messages to
  it. Use `Arc<Mutex<_>>` only when there is no other option, and **never** in
  audio callbacks.
- **RAII for cleanup.** Dropping `Engine` stops streams, joins threads, sends
  Bye and unregisters mDNS. No "remember to call stop()".
- **Small public API.** Default to `pub(crate)`. Re-export only what UIs need.

### 5.3 Errors and logging

- **Libraries** (`core`, `engine`) define error enums with **`thiserror`**.
  **Binaries** (`tui`, `app`) use **`anyhow`** with `.context(...)`.
- **No `unwrap()`/`expect()` in non-test code.** Exception: a provable
  invariant, written as `expect("reason this can't fail")` with a lint allow
  and a comment.
- **Logging uses `tracing`.**
  - TUI: logs go to a file in the platform log directory (`directories` +
    `tracing-appender`) and to an in-app log panel.
  - Android: logs go to logcat.
  - Never log passphrases, keys or audio content.
- **Panics restore the terminal.** `ratatui::init()` installs a panic hook that
  does this.

### 5.4 Real-time audio rules

Inside a cpal callback:

- ❌ No `Mutex` or `RwLock`, no `Vec::new`/`collect`/`format!`, no logging, no
  socket calls, no `try_clone()`, no blocking channel sends.
- ✅ Only: read and write `rtrb` ring buffers, atomics, and arithmetic on
  pre-allocated buffers.

### 5.5 Testing

| Level | What | Tools |
|---|---|---|
| Unit | Every function in `takkie-core` | `#[test]`, `cargo nextest` |
| Property | Packet encode/decode round-trips, jitter buffer invariants, crypto round-trips | `proptest` |
| Fuzz | Packet parser and decryptor (they read untrusted network data) | `cargo-fuzz`, scheduled CI job |
| Integration | Two `Engine`s on loopback with fake audio: discovery, talk, channels, wrong passphrase, packet loss | `tests/` in `takkie-engine`, runs on all 3 OSes in CI |
| UI | TUI screens rendered to a test backend and compared | ratatui `TestBackend` + `insta` snapshots |
| Manual | Real devices: Win ↔ macOS ↔ Linux ↔ Android, Bluetooth headset, 44.1 kHz mic, guest Wi-Fi | Release checklist (E18) |

**Coverage target:** at least 90% for `takkie-core`, measured with
`cargo llvm-cov`. No target for the I/O crates. They are covered by the
integration tests instead.

### 5.6 Supply chain and docs

- **`cargo-deny`** checks licences (allowlist), security advisories, banned
  crates and duplicate versions in CI.
- **Dependabot** updates Cargo dependencies and GitHub Actions.
- Every public item in `core` and `engine` has a rustdoc comment. Each crate
  has a short README. Decisions go in `docs/adr/`.

---

## 6. Branching, CI and releases

### 6.1 Branch model

```text
feat/E5.2-packet-header ──PR──► develop ──release PR──► main ──tag v0.2.0──► GitHub Release
fix/E7.3-ipv4-pick      ──PR──┘                           ▲
hotfix/0.2.1-crash      ─────────────────PR───────────────┘   (then merge main back into develop)
```

- **`develop`** is the default branch. All work goes here through PRs.
  Protected: CI must pass. There is no required review, because GitHub doesn't
  let you approve your own PR. Instead, each PR uses a self-review checklist
  (tests added, docs updated, real-time audio rules respected).
- **`main`** is for releases only. It receives merges from `develop` (or
  hotfixes). Protected.
- **Branch names:** `feat/`, `fix/`, `chore/`, `docs/` + ticket id + short
  name.
- **Commit messages:** Conventional Commits (`feat:`, `fix:`, `chore:`,
  `docs:`, `refactor:`, `test:`). The changelog is generated with `git-cliff`.

### 6.2 CI (`.github/workflows/ci.yml`, on every PR and push to develop/main)

| Job | Runs on | Steps |
|---|---|---|
| `check` | Ubuntu, Windows, macOS | `fmt --check` · `clippy -D warnings` · `nextest` · build `takkie` |
| `msrv` | Ubuntu | `cargo check` with Rust 1.88 |
| `deny` | Ubuntu | `cargo deny check` |
| `gui` | Ubuntu, Windows, macOS | Build the Tauri app (only when `apps/` changes; from M3) |
| `android` | Ubuntu | Build the APK (from M4) |
| `fuzz` | Ubuntu, nightly schedule | 10 minutes per fuzz target |

The Linux runner installs `libasound2-dev` (plus WebKitGTK for the `gui` job
only). `CMAKE_POLICY_VERSION_MINIMUM=3.5` comes from the committed
`.cargo/config.toml` (E1.8), so CI and developers get it automatically.

### 6.3 Release (`.github/workflows/release.yml`, on tag `v*` on `main`)

| Artifact | Tool | Formats |
|---|---|---|
| Terminal app | `dist` (cargo-dist) | `.zip`/`.tar.gz` for Windows x64, macOS arm64 + x64, Linux x64 + arm64; shell/PowerShell installers |
| Desktop GUI | `tauri-action` | Windows `.msi`/`.exe`, macOS `.dmg`, Linux `.AppImage`/`.deb` |
| Android | `tauri android build` | `.apk` signed with our own key (GitHub Release) |

Libopus is linked statically, so **end users need nothing installed.**

**No paid code signing for now** (decision 4). This is what users will see:

- **Windows:** SmartScreen says "unknown publisher". The user clicks
  *More info → Run anyway*.
- **macOS:** Gatekeeper blocks the first launch. The user right-clicks the app
  and chooses *Open*, or uses *System Settings → Privacy & Security →
  Open Anyway*.
- **Linux:** no warning.
- **Android:** APKs must always be signed, but with a free key we generate
  ourselves, not a paid certificate. Users allow "install unknown apps" once.

The user guide (E18.2) explains these steps. Paid signing (Apple Developer
account, Windows certificate) and store releases come at the end, when the
apps are ready.

**Versioning:** SemVer. Before 1.0, a minor bump (0.2 → 0.3) may change the
protocol, and the protocol `version` field stops old and new apps from
misreading each other. From 1.0, protocol changes must be backward compatible.

---

## 7. Milestones and epics

Sizes are in focused hours for one developer: **S** ≈ 4–8 h · **M** ≈ 16–30 h
· **L** ≈ 40–80 h · **XL** ≈ 80–160 h. The hours used for the schedule are in
[section 8.2](#82-estimated-schedule).

Items marked **(stretch)** are nice to have. They are not counted in the
schedule and are the first to be cut or moved after v1.0.

When tickets are created, any work item bigger than about 8 hours is split
into smaller tickets. Each ticket should then fit in two weekday sessions or
one weekend.

### M0 — Foundation → `v0.1.0`

**Goal:** a clean repo that builds on all 3 desktop OSes, with green CI and the
standards from section 5 in place. Behaviour stays the same apart from the
rename.

#### E1 — Repo cleanup and rename · M

| Item | Work |
|---|---|
| E1.1 | Start from `develop` (it already contains everything on `main`). Keep its library split and the tests that test our own code. Drop tests that only test the standard library, and any unused dev-dependencies. |
| E1.2 | Rename: crate `local-takkie`, binary `takkie`, all "VideoLAN"/"vl" text. |
| E1.3 | Remove the 22 unused dependencies. Check that Linux no longer needs GTK/WebKit. |
| E1.4 | Delete `cargo_tool.exe`, `cargo_tool.rs`, `src/discovery.rs`, `setup.sh`, `run_tests.sh`, `scripts/` (replaced by xtask in E2) and the outdated `AUDIO_STREAMING.md`. Tidy `.gitignore`. |
| E1.5 | Run `cargo fmt` and fix all clippy warnings. |
| E1.6 | Close PR #3, delete stale branches, make `develop` the GitHub default branch. (Branch protection comes in E3.4, once the new CI exists.) |
| E1.7 | Rewrite the README: what it is, build prerequisites per OS (CMake, `libasound2-dev`), usage. |
| E1.8 | Windows build fixes found in the baseline: add `.cargo/config.toml` with `[env] CMAKE_POLICY_VERSION_MINIMUM = "3.5"` (so nobody has to set it by hand) and `[profile.dev.package.audiopus_sys] opt-level = 1` in `Cargo.toml`. |

**Done when:** a fresh clone builds on Windows, macOS and Linux by following
the README, and `cargo machete` reports no unused dependencies.

#### E2 — Workspace and engineering standards · M

| Item | Work |
|---|---|
| E2.1 | Convert to a workspace: `crates/takkie-core`, `crates/takkie-engine`, `crates/takkie-tui` (move the current code into engine/tui). |
| E2.2 | `[workspace.package]`, `[workspace.dependencies]`, `[workspace.lints]`, `rust-version = "1.88"`, release profile. |
| E2.3 | `xtask` crate: `cargo xtask ci` runs fmt-check, clippy, tests and deny locally on any OS. |
| E2.4 | `deny.toml` (licences, advisories, bans). |
| E2.5 | `docs/adr/` with ADRs D1–D10 from section 4. |
| E2.6 | `CONTRIBUTING.md` (branch flow, commit style, how to run checks, real-time audio rules) and a PR template with the self-review checklist. |

**Done when:** `cargo xtask ci` passes on Windows, macOS and Linux.

#### E3 — CI/CD baseline · M

| Item | Work |
|---|---|
| E3.1 | Replace the old workflow with `ci.yml` (`check`, `msrv`, `deny` jobs from 6.2), with caching (`Swatinem/rust-cache`). |
| E3.2 | `release.yml` with cargo-dist for the terminal app. |
| E3.3 | Dependabot for cargo and actions. |
| E3.4 | Branch protection requires CI on `develop` and `main`. |
| E3.5 | `CHANGELOG.md` generated with `git-cliff` from Conventional Commits. |
| E3.6 | Release `v0.1.0`: release PR `develop → main`, tag, check the downloads on all 3 OSes. |

**Done when:** CI is green on `develop`, and tagging `v0.1.0` on `main`
publishes terminal binaries for all 3 OSes.

---

### M1 — Reliable desktop terminal app → `v0.2.0`

**Goal:** the terminal app works reliably between Windows, macOS and Linux, on
any audio device, with several talkers. This is the first release worth using.

#### E4 — Android feasibility spike · M (time-boxed: about 25 hours, one calendar week)

Proves the risky parts of Android **before** the engine design is final.

| Item | Work |
|---|---|
| E4.1 | Tauri 2 Android "hello world" builds and runs on a real phone. |
| E4.2 | cpal records from the mic and plays to the speaker on the phone (AAudio). |
| E4.3 | libopus cross-compiles for `aarch64-linux-android` with the NDK. Also compare the Opus bindings: current `opus` 0.3 / `audiopus_sys`, `opus` 0.4, and any that build without CMake. Pick one for all platforms. |
| E4.4 | mdns-sd discovers a laptop from the phone, with `MulticastLock` from a Kotlin plugin. |
| E4.5 | A foreground service keeps audio running with the screen off. |
| E4.6 | Write an ADR with the results: go, or adjust D1/D3. |

**Done when:** a phone and a laptop exchange audio (hacky code is fine), and
the ADR is written.

#### E5 — `takkie-core`: protocol, jitter buffer, DSP · L

| Item | Work |
|---|---|
| E5.1 | Newtypes: `ChannelId` (1–10), `PeerId`, `Seq`, `Passphrase`, with validation and tests. |
| E5.2 | Packet header v1 encode/decode (section 3.5), allocation-free, with typed errors. Write `docs/protocol.md`. |
| E5.3 | Property tests (round-trip) and a `cargo-fuzz` target for the decoder. |
| E5.4 | Jitter buffer: per sender, reorder, drop duplicates and late packets, target/max delay, report losses (for PLC/FEC). |
| E5.5 | DSP helpers: downmix, upmix, gain, soft limiter, RMS/peak level. |
| E5.6 | PTT state machine: hold mode, toggle mode, release-timeout fallback. Pure and fully tested. |
| E5.7 | Peer table: add, update, expire by time. Pure and tested. |
| E5.8 | Measure coverage with `cargo llvm-cov` and add it to CI. |

**Done when:** at least 90% coverage on `takkie-core`, and the fuzzer runs 10
minutes with no crash.

#### E6 — Audio engine rewrite · XL

| Item | Work |
|---|---|
| E6.1 | Device listing and selection by name (`--list-devices`, `--input-device`, `--output-device`). |
| E6.2 | Config negotiation: prefer 48 kHz/f32, otherwise use the device default. Support all sample formats. |
| E6.3 | Resampling with `rubato` in both directions when the device is not 48 kHz. |
| E6.4 | Real-time-safe input and output callbacks with `rtrb` (rules in 5.4). |
| E6.5 | tx thread: 20 ms frames, PTT gate, Opus encoder (mono, VoIP, 24 kbit/s, FEC), level meter. |
| E6.6 | mix thread: one Opus decoder per sender, PLC/FEC from the jitter buffer, mixing, volume, mute, half-duplex. |
| E6.7 | Device unplugged: emit an event and reopen the default device. |
| E6.8 | `AudioSource`/`AudioSink` traits with fake implementations for tests. |
| E6.9 | Manual check on real devices (30-minute call, 44.1 kHz mic, Bluetooth headset, two talkers). |

**Done when:**
- A 30-minute call between two machines has no growing delay.
- It works with a 44.1 kHz mic and a Bluetooth headset.
- Two people talking at once sound clear.

#### E7 — Network and discovery rewrite · L

| Item | Work |
|---|---|
| E7.1 | UDP transport: port 0 by default, socket buffer sizes via `socket2`, blocking receive with a timeout (no busy loop), send only to same-channel peers. |
| E7.2 | One mdns-sd daemon: `_takkie._udp`, TXT record, `enable_addr_auto()`, keep the handle, unregister on shutdown. |
| E7.3 | Handle resolve/remove events, pick a usable IPv4 address, filter self by `id`. |
| E7.4 | Hello/Bye packets, and peer expiry after 10 s of silence. |
| E7.5 | `--peer <ip:port>` manual peers. |
| E7.6 | Integration test: two engines on loopback with fake audio, in CI on all 3 OSes. |
| E7.7 | Manual check: join/leave timing, and machines with VPN/WSL/Docker adapters. |

**Done when:**
- Peers appear within 3 s and disappear within 10 s after a quit or crash.
- VPN, WSL and Docker adapters don't break discovery.

#### E8 — Engine API, errors, logging, shutdown · M

| Item | Work |
|---|---|
| E8.1 | Public `Engine` API (section 3.2), with docs. |
| E8.2 | `thiserror` error types. Remove every `unwrap`/`expect` from non-test code. |
| E8.3 | `tracing` with file logs in the platform log directory, `--log-level`/`RUST_LOG`. |
| E8.4 | Clean shutdown on `Drop`: stop streams, join threads, Bye, mDNS unregister. |
| E8.5 | Settings file (TOML in the platform config directory): name, channel, devices, PTT mode. |

**Done when:**
- Quitting leaves no running threads.
- "No microphone" or "port in use" shows a clear message instead of a crash.

#### E9 — Terminal UI rewrite · M

| Item | Work |
|---|---|
| E9.1 | Upgrade to current ratatui/crossterm. Use `ratatui::init()`/`restore()` (panic-safe). |
| E9.2 | clap CLI (section 3.8). The name defaults to the hostname. |
| E9.3 | PTT: detect key-release support, use hold mode if available, toggle otherwise. Show the mode. |
| E9.4 | Screens: peers (name, channel, talking), real mic level meter, buffer in ms, log panel, help popup. |
| E9.5 | Snapshot tests with `TestBackend` + `insta`. |
| E9.6 | Manual check in Windows Terminal, macOS Terminal/iTerm2, GNOME Terminal and kitty. |
| E9.7 | Release `v0.2.0` (after the `alpha` pre-releases). |

**Done when:** it is usable in Windows Terminal, macOS Terminal/iTerm2, GNOME
Terminal and kitty.

---

### M2 — Channels and privacy → `v0.3.0`

**Goal:** 10 channels with optional passphrase encryption, and walkie-talkie
behaviour.

#### E10 — Channels 1–10 · M

| Item | Work |
|---|---|
| E10.1 | Channel in config, CLI, mDNS TXT record. Re-announce when it changes. |
| E10.2 | Send only to same-channel peers. Receivers drop other channels. |
| E10.3 | TUI: keys `1`–`0` switch channel; the peer list shows each peer's channel; "channel busy" indicator. |
| E10.4 | Integration tests: users on different channels never hear each other. |

#### E11 — Passphrase encryption · M

| Item | Work |
|---|---|
| E11.1 | Argon2id key derivation and ChaCha20-Poly1305 seal/open in `takkie-core` (section 3.6), with `zeroize`. |
| E11.2 | Nonce construction and a per-sender replay window. |
| E11.3 | Passphrase entry in the TUI (masked input, never logged). Optionally remember it in the OS keychain (`keyring`). |
| E11.4 | "Wrong passphrase" detection, shown in the UI. |
| E11.5 | `docs/security.md` threat model, plus fixed test vectors so other implementations can check themselves. |

#### E12 — Walkie-talkie experience · M

| Item | Work |
|---|---|
| E12.1 | Half-duplex: mute your playback while you transmit (configurable). |
| E12.2 | "Who is talking" from the end-of-transmission flag and the talk start/stop events. |
| E12.3 | (stretch) Optional start/end beeps ("roger beep"). |
| E12.4 | Volume, mute, and per-peer mute. |

**M2 done when:** two groups on different channels at the same time, one with
a passphrase, never hear each other. A third person on the private channel
with the wrong passphrase sees "wrong passphrase" and hears nothing.

---

### M3 — Desktop GUI → `v0.4.0`

**Goal:** a desktop app that non-technical users can install and use.

#### E13 — Tauri desktop app · L

| Item | Work |
|---|---|
| E13.1 | Scaffold `apps/takkie-app` (Tauri 2 + Svelte + TypeScript) using `takkie-engine`. |
| E13.2 | Tauri commands for each Engine method, and an event bridge (`EngineEvent` → frontend). |
| E13.3 | Main screen: channel selector 1–10, big PTT button, peers with talking indicator, level meter. |
| E13.4 | Settings screen: name, devices, passphrases, PTT key, half-duplex, beeps. |
| E13.5 | PTT: mouse/touch hold, and keyboard key-down/key-up. Optional global hotkey. |
| E13.6 | (stretch) Tray icon and "keep running in background". |
| E13.7 | Frontend tests (`vitest`) and tests for the Rust commands. |

#### E14 — Desktop packaging · S–M

| Item | Work |
|---|---|
| E14.1 | Windows installer (NSIS/MSI), unsigned for now. |
| E14.2 | macOS `.dmg` with the Info.plist keys (`NSMicrophoneUsageDescription`, `NSLocalNetworkUsageDescription`, `NSBonjourServices`). Ad-hoc signed only, no Apple Developer account yet. |
| E14.3 | Linux `.AppImage` and `.deb`. |
| E14.4 | `release.yml`: build the GUI with `tauri-action` on tag. |
| E14.5 | (stretch) Auto-update with `tauri-plugin-updater`. It needs its own free update-signing key, not a certificate. |
| E14.6 | (deferred until the end) Paid code signing and macOS notarization. |

**M3 done when:** someone can download the installer, get past the OS warning
using the user-guide steps, and talk to a terminal-app user with no other
setup.

---

### M4 — Android → `v0.5.0`

**Goal:** the same app on Android phones, talking to desktop users.

#### E15 — Android app · XL

| Item | Work |
|---|---|
| E15.1 | Enable the Android target in the Tauri app: minSdk 26, ABIs `arm64-v8a`, `armeabi-v7a`, `x86_64`. |
| E15.2 | Kotlin plugin: permissions, `MulticastLock`, microphone foreground service, audio routing (speaker/earpiece/Bluetooth). |
| E15.3 | Mobile layout: big PTT button, channel picker, haptic feedback, keep-screen-on option. |
| E15.4 | Lifecycle: app in background, screen off, Wi-Fi network change (restart discovery). |
| E15.5 | Battery and CPU check (no busy loops, sensible wake locks). |
| E15.6 | (stretch) Volume key or Bluetooth headset button as PTT. |

#### E16 — Android release pipeline · M

| Item | Work |
|---|---|
| E16.1 | CI job: Android SDK/NDK, Java 17, build the APK. |
| E16.2 | Signing keystore in GitHub secrets. Signed APK in GitHub Releases. |
| E16.3 | (deferred until the end) Play Store: `.aab`, listing, privacy policy, foreground-service declaration. |

**M4 done when:** a phone on the same Wi-Fi as a laptop talks both ways on
channels 1–10, with and without a passphrase, including with the screen off.

---

### M5 — Hardening → `v1.0.0`

#### E17 — Quality and performance · L

| Item | Work |
|---|---|
| E17.1 | Scheduled fuzzing in CI. |
| E17.2 | Packet-loss and jitter simulation in integration tests (lossy fake transport). |
| E17.3 | Soak test: 4 hours, 3 peers, no memory or delay growth. |
| E17.4 | Measure end-to-end delay (target < 150 ms on LAN) and CPU use on desktop and phone. |
| E17.5 | 10 peers on one channel. |

#### E18 — Docs and release readiness · M

| Item | Work |
|---|---|
| E18.1 | User guide: install, first use, channels, passphrases. |
| E18.2 | Troubleshooting: firewall (Windows/Linux), macOS and Android permissions, guest Wi-Fi blocking discovery. |
| E18.3 | Manual test matrix and release checklist (every OS pair, Bluetooth, 44.1 kHz device, guest Wi-Fi). |
| E18.4 | Freeze protocol v1 and finalise `docs/protocol.md` and `docs/security.md`. |
| E18.5 | CHANGELOG, screenshots, README polish. |

### After v1.0 (backlog, not planned yet)

iOS app (see D5 and assumption A1) · noise suppression (`nnnoiseless`) ·
recording · text messages · Bluetooth PTT accessories · IPv6 · broadcast
discovery fallback · desktop-to-desktop over VPN.

---

## 8. Order of work and schedule

### 8.1 Order of work

```text
              ┌──────────── M0 Foundation (E1, E2, E3) ────────────┐
              │                                                     │
              ▼                                                     ▼
   M1 Desktop terminal app                                  E4 Android spike
   E5 core ──► E6 audio ─┐                                   (1 week, parallel)
          └──► E7 network ┼──► E8 engine API ──► E9 TUI            │
                          │                                         │
              ┌───────────┘   v0.2.0                                │
              ▼                                                     │
   M2 Channels + privacy (E10, E11, E12)   v0.3.0                   │
              │                                                     │
              ▼                                                     │
   M3 Desktop GUI (E13, E14)   v0.4.0  ◄── uses spike results ──────┘
              │
              ▼
   M4 Android (E15, E16)   v0.5.0
              │
              ▼
   M5 Hardening (E17, E18)   v1.0.0
```

**Why this order**

- **M0 first:** nothing else can be trusted until CI is green and the code is
  structured.
- **E5 (pure core) before E6/E7:** the protocol and jitter buffer are the
  foundation of both audio and network. They are also the easiest parts to get
  fully right with tests.
- **E4 spike early:** if Android forces a change of UI framework or codec, we
  want to know before building the GUI.
- **Channels and encryption before the GUI:** the protocol is complete before
  more apps depend on it.
- **Terminal app released first (v0.2.0):** real users can test the engine
  while the GUI is being built.

### 8.2 Estimated schedule

**Assumptions**

- One developer, about **25 focused hours a week**. That is 3–4 hours on
  weekdays (about 17 h) plus about 8 h at weekends, after allowing for
  breaks and admin.
- Start: Monday **2026-10-12**.
- Stretch and deferred items are not included.

**Hours per epic** (rough, refined when tickets are written)

| Milestone | Epics (hours) | Total |
|---|---|---|
| M0 Foundation | E1 (20) · E2 (24) · E3 (20) | **64 h** |
| M1 Desktop terminal app | E4 spike (25) · E5 core (60) · E6 audio (120) · E7 network (60) · E8 engine API (25) · E9 TUI (30) | **320 h** |
| M2 Channels + privacy | E10 (20) · E11 (30) · E12 (20) | **70 h** |
| M3 Desktop GUI | E13 (70, includes learning Tauri + Svelte) · E14 (12) | **82 h** |
| M4 Android | E15 (120) · E16 (16) | **136 h** |
| M5 Hardening | E17 (50) · E18 (30) | **80 h** |
| **Total** | | **752 h ≈ 30 weeks** |

**Calendar**

Solo projects almost always meet surprises, so plan with the "+25%" column.
The "no slack" column is the best case.

| Release | Weeks (no slack) | Ready by (no slack) | Ready by (+25%) |
|---|---|---|---|
| `v0.1.0` Foundation | ~2.5 | end of Oct 2026 | early Nov 2026 |
| `v0.2.0` Desktop terminal app | ~13 | end of Jan 2027 | end of Feb 2027 |
| `v0.3.0` Channels + privacy | ~3 | mid Feb 2027 | mid/late Mar 2027 |
| `v0.4.0` Desktop GUI | ~3.5 | mid Mar 2027 | mid Apr 2027 |
| `v0.5.0` Android | ~5.5 | mid Apr 2027 | early Jun 2027 |
| `v1.0.0` | ~3 | mid May 2027 | early Jul 2027 |

**M1 is the longest milestone (about 3–4 months).** That is where the engine
is rebuilt, and most of the risk lives there. To keep motivation and feedback
going, publish **pre-releases** along the way:

- `v0.2.0-alpha.1` after E5 + E6, plus the basic UDP path from E7.1: new
  audio engine with a basic UI.
- `v0.2.0-alpha.2` after the rest of E7: new discovery.

Re-check the schedule at the end of every milestone, and move the dates
rather than cutting tests or quality.

### 8.3 Working rhythm (solo)

- **One epic in progress at a time.** The only exception is the E4 spike,
  which can be done in any free week during M1.
- **Weekday sessions (3–4 h):** small tickets (S), reviews, docs, fixing CI.
- **Weekends:** the deep-focus work that needs long blocks (audio engine,
  jitter buffer, Android plugin).
- **Every ticket ends with a merged PR into `develop`,** even if small. Never
  leave work only on your machine.
- **At the end of each milestone:** release PR `develop → main`, tag, write
  the changelog, and update this roadmap (dates, scope, lessons learned).

---

## 9. Risks and how we reduce them

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Android kills or pauses audio in the background | High | High | Microphone foreground service (E4.5, E15.2); proven in the spike |
| libopus doesn't cross-compile for Android | Medium | High | Spike E4.3; fallbacks: prebuilt libopus per ABI, another binding |
| Networks that block multicast (guest Wi-Fi, "AP isolation") | Medium | Medium | Manual peers (E7.5); clear troubleshooting docs |
| libopus build problems on Windows (CMake 4 rejects it; debug build fails to link) | **Confirmed** | Medium | Both fixed with config (E1.8, verified). Revisit the Opus binding in E4.3, ideally one that doesn't need CMake. |
| Terminals that don't report key release | Certain (most terminals) | Low | Toggle mode fallback (E9.3); GUI apps don't have this problem |
| Variety of Linux audio stacks (ALSA/PulseAudio/PipeWire) | Medium | Medium | Device negotiation + resampling (E6.2–E6.3); test matrix |
| Echo when using speakers | Medium | Medium | Half-duplex by default (D9) |
| Unsigned builds show OS warnings | Certain | Low | Decided: unsigned until the end; steps explained in the user guide (E18.2) |
| Solo developer with limited hours: burnout, long gaps, scope creep | Certain | High | One epic at a time; small tickets; cut stretch items first; 25% buffer; the terminal app is useful on its own at v0.2.0; pre-releases inside M1 |
| iOS multicast entitlement | High (if iOS) | High | iOS is after v1.0; may need native Bonjour via a Swift plugin |
| Scope too big (3 apps, 4+ platforms) | Medium | High | Milestones ship value step by step; the terminal app is useful on its own at v0.2.0 |

---

## 10. Decisions and assumptions

**Decided by the project owner**

| # | Topic | Decision | Date |
|---|---|---|---|
| 1 | Apps | Desktop terminal app + desktop GUI + Android app, one shared Rust engine | 2026-10-07 |
| 2 | Branches | `develop` = development, `main` = releases only | 2026-10-07 |
| 3 | Name | Crate `local-takkie`, binary `takkie`; drop "VideoLAN"/"vl" | 2026-10-07 |
| 4 | Code signing | None for now. Build and release first; signing and app stores come last. | 2026-10-07 |
| 5 | Channels | 10 channels, optional passphrase encryption | 2026-10-07 |
| 6 | PTT | Toggle fallback where hold isn't possible; half-duplex by default | 2026-10-07 |
| 7 | GUI frontend | Svelte + TypeScript | 2026-10-07 |
| 8 | Ticket tracker | GitHub Issues + Milestones + a GitHub Project board | 2026-10-07 |
| 9 | Team and time | Solo developer, planned at ~25 h/week | 2026-10-07 |

**Assumptions** (change them here if they are wrong)

| # | Topic | Assumption |
|---|---|---|
| A1 | iOS | After v1.0 |
| A2 | Distribution | GitHub Releases only until the end. Stores (Play Store, winget, Homebrew) are decided after v1.0. |
| A3 | Group size | Up to 10–16 people on one channel |
| A4 | Passphrases | Optional and separate for each channel |
| A5 | Network | IPv4 LAN or Wi-Fi; no internet relay |

---

## 11. Glossary

| Term | Meaning |
|---|---|
| **PTT** | Push-to-talk: you only transmit while the button is held (or toggled on). |
| **mDNS** | Multicast DNS: devices announce themselves on the LAN so others find them without a server (like Bonjour/AirDrop discovery). |
| **TXT record** | Small key=value data attached to an mDNS announcement (we put name, id, channel there). |
| **Opus** | Audio codec designed for voice and music over networks. |
| **Jitter buffer** | Short queue that holds incoming packets a few milliseconds so they can be played at an even pace, even if the network delivers them unevenly. |
| **PLC** | Packet loss concealment: the decoder "invents" a plausible sound for a missing packet. |
| **FEC** | Forward error correction: each packet carries a low-quality copy of the previous one, so a single lost packet can be rebuilt. |
| **SPSC ring buffer** | Single-producer, single-consumer queue that never blocks or allocates. Safe to use in audio callbacks. |
| **Real-time callback** | The function the OS calls to get or give audio. If it is slow, you hear clicks. |
| **AEAD** | Authenticated encryption (here ChaCha20-Poly1305): hides content and detects tampering. |
| **KDF** | Key derivation function (here Argon2id): turns a passphrase into an encryption key, slowly on purpose to make guessing expensive. |
| **MSRV** | Minimum supported Rust version. |
| **ADR** | Architecture Decision Record: a short file explaining one technical decision and why. |
| **Foreground service** | Android component that keeps an app running (with a notification) when it is not on screen. |
| **MulticastLock** | Android switch that lets an app receive multicast (mDNS) packets on Wi-Fi. |
| **Half-duplex** | Only one direction at a time: while you talk, you don't hear others. |
