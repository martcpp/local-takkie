# Changelog

All notable changes to local-takkie. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/).

## [0.2.0-alpha.1] - 2026-10-10

### Added

- Add ChannelId, PeerId, Seq and Passphrase
- Protocol v1 header types and encoder
- Protocol v1 decoder with typed errors
- Jitter buffer storage with reorder, dedupe and late drops
- Jitter buffer playout timing, delay limits and resets
- Jitter buffer FEC/PLC hints, stats and property tests
- DSP downmix, upmix and gain without allocation
- DSP mixing, soft limiter and level meter
- Push-to-talk state machine with hold, toggle and timeout modes
- Peer table with join, update and expiry events
- Audio I/O traits and fake devices for tests
- Negotiate stream config, preferring 48 kHz f32
- Resample capture audio to 48 kHz frames with rubato
- Resample 48 kHz frames to the playback device rate
- Real-time-safe input callback feeding an rtrb ring
- Real-time-safe output callback fed from an rtrb ring
- Open streams in every cpal sample format
- Tx thread with 20 ms framing and a push-to-talk gate
- Encode tx frames with Opus at 24 kbit/s with in-band FEC
- Lock-free mic level meter fed by the tx thread
- Per-sender Opus decoders fed from jitter buffers
- List audio input and output devices
- Open audio devices by name with a fallback to the default
- Mix thread paced by the speaker's ring fill level
- Cover lost packets with Opus FEC and concealment
- Mixer volume, mute, limiter and talking detection
- Half-duplex, muting playback while transmitting
- Recover from a lost audio device on the default one
- UDP transport with blocking receive and an in-memory fake
- Rx thread that decodes, filters and routes packets
- Send path to same-channel peers via an arc-swap snapshot
- Announce _takkie._udp over mDNS with one daemon and a TXT record
- Unregister from mDNS and stop the daemon cleanly on drop
- Browse mDNS for peers and apply peer news to the peer table
- Choose a reachable peer address and skip our own mDNS record
- Send Hello every 2 s and Bye on shutdown
- Peer thread that expires quiet peers and frees their decoders
- Send Hello and Bye to static peers so mDNS isn't required
- Public Engine API with config, start and commands
- Engine events and a lock-free snapshot for UIs
- Speaker level in the snapshot, measured after volume and mute
- Run the terminal app on the Engine and remove the prototype
- Tracing in the engine and daily log files for the terminal app
- Remember name, channel and devices in a settings file
- Join engine threads with a timeout on shutdown
- Command-line flags with clap
- Ratatui 0.30 with panic-safe terminal setup
- Detect key-release support and turn it on where the terminal allows
- Push-to-talk through the PTT state machine, with hold or toggle
- Main layout with header and peer list from the engine snapshot
- Level meters, buffer delay and a log panel fed by tracing
- Help popup and keys for mute, volume and device info

### Fixed

- Install cargo-deny as a binary instead of the Docker action
- Keep peers known across channels so audio resumes after a switch
- No audio after Bye, and late packets after Bye don't bring a peer back

### Changed

- Cut comments down to what matters
- Cut the peer table doc comments down
- Cut the PTT doc comments down

### Documentation

- ADR 0011 records the Android spike as a go
- Protocol v1 spec in docs/protocol.md
- How to check mDNS discovery on a real network
- Takkie flags in README and CLAUDE.md; fix the merged lock file
- README for the rebuilt terminal app, with a terminal compatibility table

### Tests

- Property tests for the protocol header
- Fuzz target for the packet decoder
- Keep the jitter proptest regression seed
- End-to-end audio pipeline with fake devices
- Two nodes on loopback exchange audio with static peers
- Give audio time to resume after switching back to the channel
- Wait for packets from background threads instead of one short receive
- Insta snapshots of the screens; wrap long log lines

### Build and CI

- Report takkie-core coverage with cargo-llvm-cov
- Fuzz the packet decoder on Ubuntu
- Build the fuzzer for the glibc target
- Create the fuzz corpus folder before running
- Keep the fuzz command on separate lines
- Run the release workflow on tags only, not on every pull request
- Restore the release workflow and dist config

### Dependencies

- Bump toml_edit from 0.23.10+spec-1.0.0 to 0.24.0+spec-1.1.0

### Maintenance

- Switch to opus 0.4 for libopus on every platform
- Drop a stray harness state file and ignore nested .claude folders
- Drop a stray blank line in Cargo.toml
- Discovery example runs the full peer stack with Hello and expiry
- Deny unwrap and expect across the workspace


## [0.1.1] - 2026-10-08

### Fixed

- Start with defaults when takkie gets no arguments

### Maintenance

- V0.1.1 (#159)


## [0.1.0] - 2026-10-08

### Added

- Add mdns

### Fixed

- Fix crate vulnerability
- Build libopus on Windows with CMake 4 and in debug builds (E1.8) (#128)

### Changed

- Refactor code
- Refactor code
- Fix formatting and clippy errors (#127)
- Move the code into takkie-engine and takkie-tui (E2.1b) (#131)

### Documentation

- Add project roadmap (#10)
- Rewrite the README (#129)
- Add ADRs for decisions D1 to D10 (#136)
- Add CONTRIBUTING.md, a PR template and issue templates (#137)
- Add a changelog generated with git-cliff (#149)

### Tests

- Prune tests that don't exercise project code (#123)

### Build and CI

- Replace the old workflows with ci.yml for all three OSes (#138)
- Run cargo-deny in CI (#139)
- Add a dist release workflow for the terminal app (#140)
- Add Dependabot for Cargo and GitHub Actions (E3.3) (#141)
- Bump actions/checkout from 6.1.0 to 7.0.1 (#144)

### Dependencies

- Bump the cargo-minor-and-patch group with 2 updates (#142)
- Bump mdns-sd from 0.17.1 to 0.20.0 (#145)
- Bump crossterm from 0.28.1 to 0.29.0 (#146)

### Maintenance

- Add Claude Code harness config (#122)
- Rename crate to local-takkie and binary to takkie (E1.2) (#124)
- Remove the 22 unused dependencies (E1.3) (#125)
- Delete obsolete files and tidy .gitignore (E1.4) (#126)
- Create the Cargo workspace and the three crates (#130)
- Share workspace metadata and lints across the crates (E2.2a) (#132)
- Add a release profile and rustfmt.toml (#133)
- Add cargo xtask for cross-platform dev commands (#134)
- Add cargo-deny for licences, advisories, bans and sources (#135)
- Add cargo xtask release to prepare a release in one command (E3.7) (#151)
- V0.1.0 (#153)
- Record the v0.1.0 squash on main as merged into develop

### Other

- Initial commit
- Added udp featuree
- Breaking thing to files
- Did refactoring
- Starting audio processing
- Add tui
- Added tui
- Added set up for other machince
- Making the inde wokr for all othe rdevice
- Uodate
- Added libatk1.0-dev
- Added some tool for linux work
- Added linux dependices
- Feature/fix missing opus dependency (#1)
- Clippy fix (#2)
- Clippy fix (#5)
- Fix audio sample rate on linux and mac (#6)
- Fix audio sample rate on linux and mac (#7)
- Added to rad.rs
- Sample rate (#8)
- Ptt fix
- Ptt issue (#9)
- Ppt issue
- Made some fix
- Setting for prod
- Added cmd for build
- V0.1.0 (#152)

