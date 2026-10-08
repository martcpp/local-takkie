# CLAUDE.md

local-takkie is a walkie-talkie for the local network: peers find each other
with mDNS, pick one of 10 channels, and talk with push-to-talk over UDP. It is
being rebuilt from a prototype into a terminal app, a desktop GUI and an
Android app.

`ROADMAP.md` is the plan and the source of truth for architecture, standards
and order of work. Read the relevant section before starting a task.

## Workflow rules

- **Tickets are the source of truth.** `ROADMAP.md` was approved on
  2026-10-07. Work is tracked as GitHub Issues (epics #11–#28, tickets under
  them) and on the [Project board](https://github.com/users/martcpp/projects/3).
  Work on one ticket at a time, in milestone order. Tickets for M2–M5 are
  created when each milestone starts. When something is unclear, ask instead
  of assuming.
- **Plans.md** only mirrors the tickets currently in progress, with the
  Harness markers (`cc:todo`, `cc:wip`, `cc:done`) and the issue number plus
  roadmap id (for example `#29 E1.1`). It never holds work that has no ticket.
- **Branches:** `develop` is the integration branch; `main` is for releases
  only. Work on `feat/`, `fix/`, `chore/` or `docs/` + ticket id + short name,
  and open PRs into `develop`.
- **Commits:** Conventional Commits (`feat:`, `fix:`, `docs:`, `chore:`,
  `refactor:`, `test:`). No AI co-author or "Generated with" lines.
- Do not commit or push unless asked.

## Build and test

Current state: a single crate `local-takkie`, binary `takkie` (`takkie <name> <port>`).
This section changes with tickets #30 (rename), #32 (scripts removed), #38
(workspace) and #41 (`cargo xtask ci`); update it in those PRs.

```bash
cargo build
cargo test
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
```

libopus is built from C source, so CMake must be installed. The CMake 4 policy
setting (`.cargo/config.toml`) and the Windows debug-runtime fix
(`[profile.dev.package.audiopus_sys]` in `Cargo.toml`) are already in the repo.
If CMake picks a Visual Studio version that isn't fully installed, set
`CMAKE_GENERATOR` (for example `Visual Studio 17 2022`).

## Code standards (summary of ROADMAP.md §5)

- No `unwrap()`/`expect()` in non-test code. Libraries use `thiserror`,
  binaries use `anyhow` with `.context(...)`. Logging uses `tracing`.
- **Real-time audio callbacks:** no locks, allocation, logging, socket calls
  or blocking sends. Only ring buffers, atomics and arithmetic on
  pre-allocated buffers.
- Logic stays pure and testable; hardware and network go behind traits
  (`AudioSource`, `AudioSink`, `Transport`) so tests need no devices.
- Never log passphrases, keys or audio content.
