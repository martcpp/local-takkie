# local-takkie

A push-to-talk walkie-talkie for your local network. Run it on two or more
computers on the same Wi-Fi or LAN and they find each other on their own. Hold
the space bar to talk to everyone else; no internet, server or account needed.

> **Status: the terminal app works; more is coming.** The audio and network
> engine has been rebuilt and the terminal app runs on it. Still to come:
> switching channels from inside the app, optional encryption, a desktop app
> and an Android app. The plan is in [ROADMAP.md](ROADMAP.md) and progress is
> on the [project board](https://github.com/users/martcpp/projects/3).

## What you need

- **The latest stable Rust**, installed with [rustup](https://rustup.rs).
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
cargo run --release -p takkie-tui
```

That's it: by default it uses your computer's name, channel 1 and any free
port. Flags change that, for example
`cargo run --release -p takkie-tui -- --name alice --channel 2`. Run
`takkie --help` for all of them (`--port`, `--input-device`,
`--output-device`, `--list-devices`, `--peer`, `--ptt-mode`, `--log-level`,
`--config`). The name, channel and devices you use are remembered for next
time. To try it on a single computer, just start two copies.

With a release download, run `takkie` (or double-click `takkie.exe` on
Windows).

## Using it

Other people show up in the peer list a few seconds after they start, with a
green dot while they talk. Press `?` in the app for the list of keys:

| Key | Does |
|---|---|
| `SPACE` | Talk. Hold it where your terminal can tell when you let go, otherwise press once to start and once to stop. The footer says which. |
| `T` | Switch between hold and toggle |
| `M` | Mute or unmute what you hear |
| `+` / `-` | Volume up or down |
| `D` | Show the microphone and speaker in use |
| `1`–`9`, `0` | Switch to channel 1 to 10 |
| `P` | Set or clear this channel's passphrase |
| `?` | Help |
| `Q` / `Esc` | Quit |

Your name, channel and devices are kept in a settings file, and the app shows
where it is when it starts. Everyone on the same channel hears each other;
choose one with the digit keys or `--channel`.

**Private channels.** A channel is open by default: anyone on the same
network can listen and talk. Press `P` and type a passphrase to make it
private; everyone who should hear you sets the same passphrase on the same
channel, and a 🔒 shows in the header. People without it hear nothing and
can't talk into it. The passphrase is never saved to disk. What this does and
doesn't protect is in [docs/security.md](docs/security.md).

## Terminals

How well `takkie` works depends on the terminal, mostly on whether it tells
the app when a key is released (that decides hold-to-talk or toggle).

| Terminal | Talk key | Colours, emoji, resizing | Checked |
|---|---|---|---|
| Git Bash (mintty), Windows 11 | Hold | Good | Yes, v0.2.0 development build |
| Windows Terminal, PowerShell, cmd | Hold expected (Windows reports key releases) | Not tested yet | No |
| VS Code terminal, Windows | Hold expected | Not tested yet | No |
| kitty, foot, Ghostty, Alacritty, WezTerm | Hold expected (kitty keyboard protocol; WezTerm needs it switched on) | Not tested yet | No |
| macOS Terminal, iTerm2, GNOME Terminal | Toggle expected (no key releases) | Not tested yet | No |

The app says what it found when it starts (`Key releases: ...`) and the
footer always shows the current mode. `T` switches, and `--ptt-mode hold` or
`--ptt-mode toggle` forces one.

## Troubleshooting

**People can't see each other.** Make sure everyone is on the same network
and the same channel, and allow the app through the firewall (Windows asks on
first run: allow it on private networks). It needs its UDP port and mDNS (UDP
5353). Guest Wi-Fi often stops devices from finding each other; if so, start
one side with the other's address, `takkie --peer 192.168.1.20:40000` (the
port is in the other app's header).

**No sound.** Run `takkie --list-devices` and pick one with `--input-device`
or `--output-device`; part of the name is enough. Press `D` in the app to see
what it's using, and watch the Mic and Speaker meters.

**It won't start.** It prints one line saying why, for example a port that's
already in use or no microphone. Warnings and errors also appear in the
app's Events Log and in the log file, whose folder is shown at startup. For
more detail, start with `--log-level debug`.

**CMake can't find Visual Studio** (Windows, building from source). If CMake
picks a Visual Studio version that isn't fully installed, point it at the one
you have, for example in PowerShell:
`$env:CMAKE_GENERATOR = "Visual Studio 17 2022"`.

## Development

Work on a branch from `develop` and open a pull request into `develop`;
`main` is only for releases. Before opening a PR, run the same checks CI
runs (formatting, clippy, tests and cargo-deny):

```bash
cargo xtask ci
```

`cargo xtask fmt` formats everything. The full workflow and code rules are in
[CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT, see [LICENSE](LICENSE).
