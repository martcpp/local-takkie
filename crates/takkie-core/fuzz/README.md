# Fuzzing takkie-core

The packet decoder reads untrusted network data, so it's fuzzed with
[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz). This needs a nightly
toolchain, so the crate here has its own workspace and the normal build never
sees it.

## Setup

```bash
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --locked
```

## Run

From `crates/takkie-core`:

```bash
mkdir -p fuzz/corpus/decode_header
cargo +nightly fuzz run decode_header fuzz/corpus/decode_header fuzz/seeds/decode_header -- -max_total_time=600
```

That runs for 10 minutes, starting from the hand-made inputs in
`fuzz/seeds/decode_header` and saving new ones to `fuzz/corpus` (not
committed). On Windows add `--sanitizer none` if AddressSanitizer isn't
available.

The target checks that decoding never panics, and that anything which decodes
encodes back to the same bytes. A crash is saved under `fuzz/artifacts`;
replay it with:

```bash
cargo +nightly fuzz run decode_header fuzz/artifacts/decode_header/<file>
```

## In CI

The `Fuzz` workflow runs it for 10 minutes on Ubuntu whenever a pull request
changes the decoder or this folder. You can also start it from the Actions tab
("Run workflow"), with a different length if you like. A crash is uploaded as
the `fuzz-artifacts` artifact.
