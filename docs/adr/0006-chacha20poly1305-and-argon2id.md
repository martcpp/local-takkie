# 0006. ChaCha20-Poly1305 and Argon2id for channel passphrases

- **Status:** Accepted
- **Date:** 2026-10-07

## Context

A channel can have an optional passphrase. People without it must not be
able to listen in or inject audio. There is no server for key exchange.

## Decision

Derive a key per channel from the passphrase with Argon2id, and seal each
packet with ChaCha20-Poly1305 (RustCrypto crates). The packet header is
authenticated data; the nonce is the sender id plus the sequence number.

## Consequences

- Fast on phones without AES hardware, and pure Rust.
- Weak passphrases can be guessed offline, so the UI warns about short ones.
- Names, channel numbers and who is talking stay visible on the LAN. The
  threat model goes in `docs/security.md` (E11.5).

## Reconsider if

We need per-user keys or forward secrecy.

Alternatives considered: AES-GCM, a PAKE-based key exchange.
