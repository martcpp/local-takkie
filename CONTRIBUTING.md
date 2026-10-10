# Contributing

Thanks for helping with local-takkie. This page covers how work flows and the
rules every change follows. The full plan and standards are in
[ROADMAP.md](ROADMAP.md).

## Setup

Install what the [README](README.md#what-you-need) lists. To run the
supply-chain checks locally as well, install cargo-deny:

```bash
cargo install --locked cargo-deny
```

## How work flows

1. **Pick a ticket.** Work is tracked as GitHub issues on the
   [project board](https://github.com/users/martcpp/projects/3), in milestone
   order. One ticket per branch and per pull request.
2. **Branch from `develop`**, named `<type>/<ticket id>-<short-name>`, for
   example `feat/E5.2a-packet-encoder`. Types: `feat`, `fix`, `chore`, `docs`,
   `refactor`, `test`.
3. **Open a PR into `develop`.** Use a Conventional Commit title and put
   `Closes #<issue>` in the description. PRs are squash-merged.
4. **Releases** happen on `main` only: a release PR from `develop` to `main`,
   then a version tag.

## Commits

Use [Conventional Commits](https://www.conventionalcommits.org/):
`feat:`, `fix:`, `docs:`, `chore:`, `refactor:`, `test:`. Keep the subject
short and in the imperative ("add packet encoder"), and use the body to say
why. No AI co-author or "Generated with" lines.

## Before every PR

```bash
cargo xtask ci
```

It runs what CI runs: formatting, clippy with `-D warnings`, the tests and
cargo-deny. It has to pass.

CI also measures test coverage of `takkie-core` (the goal is 90% of lines).
To see it locally:

```bash
rustup component add llvm-tools-preview
cargo install --locked cargo-llvm-cov
cargo llvm-cov -p takkie-core --summary-only
```

## The desktop app

The GUI is in `apps/takkie-app`: a Tauri 2 app with the Rust side in
`src-tauri` and a Svelte + TypeScript frontend in `ui`. It needs
[Node.js](https://nodejs.org) 22 or newer and the Tauri CLI
(`cargo install --locked tauri-cli`). On Linux it also needs the
[WebKitGTK packages Tauri lists](https://tauri.app/start/prerequisites/#linux).

```bash
cd apps/takkie-app
npm --prefix ui install     # once
cargo tauri dev             # opens the window and reloads on changes
```

`cargo xtask ci` leaves the app out, so the terminal app can be worked on
without Node or WebKitGTK. When you change anything under `apps/`, also run:

```bash
cargo xtask gui
```

It type-checks and builds the frontend, then runs clippy and the tests for
the app.

## Checking discovery on a real network

The tests never touch real mDNS, because CI runners may block multicast.
Two engines talking over 127.0.0.1 with static peers is covered by
`crates/takkie-engine/tests/two_nodes.rs`, which runs on all three OSes in
CI. To check mDNS itself, run the `discovery` example in two terminals, or
on two machines on the same network:

```bash
cargo run -p takkie-engine --example discovery -- Kitchen 2 30
cargo run -p takkie-engine --example discovery -- Bedroom 2 30
```

Each one should print `joined` for the other within a few seconds, and
`left` within about a second of the other one exiting. If nothing shows up,
check that the firewall allows UDP port 5353 and that both are on the same
subnet.

## Code rules

The short version of [ROADMAP.md section 5](ROADMAP.md#5-rust-engineering-standards):

- **Don't silence checks to get green.** No new `#[allow]`, looser lints or
  `#[ignore]` on a test. Fix the cause, or explain in the PR what's blocking.
- **Errors, not panics.** No `unwrap`/`expect` in non-test code (`takkie-core`
  enforces it now, the whole workspace from E8.2). Libraries return typed
  errors with `thiserror`; binaries use `anyhow` with context.
- **Tests.** A behaviour change comes with a test, a bug fix with a test that
  fails without it. Logic lives in `takkie-core` as plain, testable code;
  hardware and network go behind traits so tests need no devices.
- **Docs.** Public items get rustdoc, with `# Errors` and `# Panics` sections
  where they apply.
- **Comments** only when they say something the code can't, like a
  non-obvious reason or a workaround. Keep them short.
- **Dependencies** used by two or more crates go in `[workspace.dependencies]`;
  one used by a single crate stays in that crate's `Cargo.toml`.
- **Rust version:** the latest stable, picked up from `rust-toolchain.toml`.

### Real-time audio

Inside an audio device callback, never lock a mutex, allocate, log, touch a
socket or block on a channel. Only copy samples to or from the ring buffers,
use atomics, and do arithmetic on buffers allocated beforehand. Anything slower
causes clicks and dropouts. See [ADR 0007](docs/adr/0007-rtrb-ring-buffers.md).

## Recording a decision

For a decision that shapes the project (a library, a protocol change, a new
rule), add an ADR: copy
[docs/adr/0000-template.md](docs/adr/0000-template.md), take the next number,
add a row to [docs/adr/README.md](docs/adr/README.md), and link it from the PR.

## Releasing

Releases are cut from `develop` into `main` and published by tagging `main`.

1. On a branch from `develop`, prepare the release, then open a PR into
   `develop`:

   ```bash
   cargo xtask release 0.2.0
   ```

   It sets `version` in `[workspace.package]`, updates `Cargo.lock` and
   regenerates `CHANGELOG.md` with git-cliff, then prints the next steps.

2. Open a release PR from `develop` into `main` and merge it with a merge
   commit (not squash), so `main` and `develop` keep the same history.
3. Tag the merge commit on `main` and push the tag. The tag has to match the
   version in `Cargo.toml`:

   ```bash
   git switch main && git pull
   git tag v0.2.0
   git push origin v0.2.0
   ```

4. Open a PR from `main` back into `develop` and merge it with a merge commit.
   It changes no files, but it puts the release commit and its tag into
   `develop`'s history; without it the next changelog would repeat everything
   since the start.

The release workflow then builds the archives and installers for every
platform and publishes the GitHub Release, using that version's section of
`CHANGELOG.md` as the release notes. Install git-cliff with
`cargo install --locked git-cliff`.
