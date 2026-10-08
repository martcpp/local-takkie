# local-takkie

A push-to-talk walkie-talkie for your local network. Run it on two or more
computers on the same Wi-Fi or LAN and they find each other on their own. Hold
the space bar to talk to everyone else; no internet, server or account needed.

> **Status: early prototype.** It works in the simple case, but it is being
> rebuilt into a terminal app, a desktop app and an Android app, with 10
> channels and optional encryption. The plan is in [ROADMAP.md](ROADMAP.md)
> and progress is on the [project board](https://github.com/users/martcpp/projects/3).

## What you need

- **Rust 1.88 or newer**, installed with [rustup](https://rustup.rs).
- **CMake**, because the Opus audio codec is compiled from C source.
- A microphone, and headphones (there is no echo cancellation yet).

Platform tools:

| Platform | Install |
|---|---|
| Windows | Visual Studio Build Tools with the "Desktop development with C++" workload, then `winget install Kitware.CMake` |
| macOS | `xcode-select --install`, then `brew install cmake` |
| Debian / Ubuntu | `sudo apt install build-essential cmake pkg-config libasound2-dev` |
| Fedora | `sudo dnf install gcc cmake pkgconf-pkg-config alsa-lib-devel` |

## Build and run

```bash
git clone https://github.com/martcpp/local-takkie.git
cd local-takkie
cargo run --release -- <your-name> <port>
```

For example, run `cargo run --release -- alice 5000` on one computer and
`cargo run --release -- bob 5000` on another. To try it on a single computer,
give each copy a different port.

## Using it

- **Hold SPACE** to talk, let go to stop.
- **Q** or **Esc** quits.
- Other people show up in the peer list a few seconds after they start.

Known limits of the prototype (all planned fixes, see the roadmap):

- Most macOS and Linux terminals don't report when a key is released, so
  talking stops a moment after the last key repeat and can cut in and out.
  Windows Terminal works properly.
- Your microphone and speakers must support 48 kHz audio.
- People who quit stay in the peer list.

## Troubleshooting

**People can't see each other.** Make sure everyone is on the same network,
and allow the app through the firewall (Windows asks on first run: allow it on
private networks). It needs your chosen UDP port and mDNS (UDP 5353). Guest
Wi-Fi often stops devices from seeing each other.

**CMake can't find Visual Studio** (Windows). If CMake picks a Visual Studio
version that isn't fully installed, point it at the one you have, for example
in PowerShell: `$env:CMAKE_GENERATOR = "Visual Studio 17 2022"`.

**No sound, or it crashes at start.** Check your default input and output
devices, and that they support 48 kHz.

## Development

Work on a branch from `develop` and open a pull request into `develop`;
`main` is only for releases. Before opening a PR, run:

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## License

MIT, see [LICENSE](LICENSE).
