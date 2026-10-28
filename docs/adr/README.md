# Architecture decisions

Short records of the decisions that shape local-takkie: what we chose, why,
and what would make us change our minds. The summary table is in
[ROADMAP.md section 4](../../ROADMAP.md#4-key-technical-decisions).

| ADR | Decision | Status |
|---|---|---|
| [0001](0001-tauri-for-desktop-gui-and-android.md) | Tauri 2 for the desktop GUI and the Android app | Accepted, confirmed by 0011 |
| [0002](0002-threads-and-channels-not-async.md) | Plain threads and channels in the engine, no async runtime | Accepted |
| [0003](0003-opus-codec.md) | Opus for voice | Accepted, `opus` 0.4 since #54 |
| [0004](0004-cpal-for-audio-io.md) | cpal for audio input and output | Accepted |
| [0005](0005-mdns-sd-for-discovery.md) | mdns-sd for finding peers | Accepted |
| [0006](0006-chacha20poly1305-and-argon2id.md) | ChaCha20-Poly1305 and Argon2id for channel passphrases | Accepted |
| [0007](0007-rtrb-ring-buffers.md) | rtrb ring buffers next to the audio callbacks | Accepted |
| [0008](0008-custom-binary-packet-header.md) | A small custom binary packet header | Accepted |
| [0009](0009-half-duplex-by-default.md) | Half-duplex by default | Accepted, confirmed by 0015 |
| [0010](0010-xtask-for-dev-automation.md) | cargo xtask for dev automation | Accepted, done in #41 |
| [0011](0011-android-feasibility.md) | Android is feasible with Tauri, cpal, Opus and mdns-sd | Accepted |
| [0012](0012-rnnoise-for-noise-suppression.md) | RNNoise (`nnnoiseless`) for noise suppression | Accepted |
| [0013](0013-deepfilternet-not-yet.md) | DeepFilterNet is better in loud rooms but is not adopted yet | Accepted |
| [0014](0014-deep-plc-after-field-data.md) | Opus's neural loss hiding helps with bursts; decided after field data | Accepted |
| [0015](0015-no-echo-cancellation-for-now.md) | Echo cancellation is possible in pure Rust, and still not built | Accepted |
| [0016](0016-our-own-voice-processing-on-android.md) | On Android, our own voice processing, not the phone's | Accepted |

To add one, copy [0000-template.md](0000-template.md), take the next number,
and add a row here.
