# Architecture decisions

Short records of the decisions that shape local-takkie: what we chose, why,
and what would make us change our minds. The summary table is in
[ROADMAP.md section 4](../../ROADMAP.md#4-key-technical-decisions).

| ADR | Decision | Status |
|---|---|---|
| [0001](0001-tauri-for-desktop-gui-and-android.md) | Tauri 2 for the desktop GUI and the Android app | Accepted, to be confirmed by the Android spike |
| [0002](0002-threads-and-channels-not-async.md) | Plain threads and channels in the engine, no async runtime | Accepted |
| [0003](0003-opus-codec.md) | Opus for voice | Accepted, binding chosen in #54 |
| [0004](0004-cpal-for-audio-io.md) | cpal for audio input and output | Accepted |
| [0005](0005-mdns-sd-for-discovery.md) | mdns-sd for finding peers | Accepted |
| [0006](0006-chacha20poly1305-and-argon2id.md) | ChaCha20-Poly1305 and Argon2id for channel passphrases | Accepted |
| [0007](0007-rtrb-ring-buffers.md) | rtrb ring buffers next to the audio callbacks | Accepted |
| [0008](0008-custom-binary-packet-header.md) | A small custom binary packet header | Accepted |
| [0009](0009-half-duplex-by-default.md) | Half-duplex by default | Accepted |
| [0010](0010-xtask-for-dev-automation.md) | cargo xtask for dev automation | Accepted, done in #41 |

To add one, copy [0000-template.md](0000-template.md), take the next number,
and add a row here.
