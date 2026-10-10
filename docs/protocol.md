# local-takkie wire protocol, version 1

This is enough to talk to local-takkie from another implementation. The Rust
code is in `crates/takkie-core/src/protocol.rs`.

## Transport

- UDP, IPv4. One packet per datagram.
- Peers find each other with mDNS (service `_takkie._udp.local.`), which
  gives each peer's address and port.
- Every packet is a 24-byte header followed by a payload.

## Header

All multi-byte fields are big-endian.

| Offset | Size | Field | Rules |
|---|---|---|---|
| 0 | 2 | `magic` | Always `54 4B` ("TK"). Drop anything else. |
| 2 | 1 | `version` | `1`. Drop other versions. |
| 3 | 1 | `kind` | `1` Audio, `2` Hello, `3` Bye. Drop other values. |
| 4 | 1 | `channel` | `1` to `10`. Drop other values. |
| 5 | 1 | `flags` | Bit 0: encrypted. Bit 1: end of transmission. Other bits are reserved: send 0, ignore on receipt. |
| 6 | 2 | `reserved` | Send `00 00`. Drop packets where it isn't. |
| 8 | 8 | `sender_id` | Random 64-bit number picked when the app starts. |
| 16 | 4 | `seq` | See below. |
| 20 | 4 | `timestamp` | See below. |
| 24 | n | `payload` | Depends on `kind`. |

A receiver checks, in this order: at least 24 bytes, magic, version, kind,
channel, reserved. The first failure drops the packet.

## Packet kinds

| Kind | Payload | When |
|---|---|---|
| Audio (`1`) | One Opus packet: mono, 48 kHz, 20 ms | About 50 a second while talking |
| Hello (`2`) | Display name, UTF-8 | Every 2 seconds |
| Bye (`3`) | Empty | Once, when the app closes |

Audio goes only to peers on the sender's channel. Hello and Bye go to every
known peer, whatever its channel, so peers keep track of each other across
channels; the header says which channel the sender is on.

A peer that sends nothing for 10 seconds is treated as gone. After a Bye,
ignore anything else from that `sender_id` for 10 seconds: it is a late
packet.

Set the end-of-transmission flag on the last Audio packet of a push-to-talk
press, so receivers can flush their jitter buffer straight away.

## Sequence numbers

- `seq` goes up by one for every packet a sender sends, of any kind.
- It wraps from `FFFFFFFF` to `00000000`.
- Compare with serial number arithmetic (RFC 1982): `a` is newer than `b` if
  `(a - b) mod 2^32` is between 1 and 2^31 - 1. Exactly 2^31 apart is neither.
- A new `sender_id` starts a new sequence.

## Timestamps

`timestamp` is the sender's audio position in 48 kHz samples, wrapping like
`seq`. Consecutive 20 ms Audio packets differ by 960. Receivers use it to
spot gaps; for Hello and Bye it carries the current position and can be
ignored.

## Encryption

A channel is either open (no passphrase) or private (everyone on it uses the
same passphrase). What this does and doesn't protect is in
[security.md](security.md). The Rust code is in
`crates/takkie-core/src/key.rs`, `seal.rs` and `replay.rs`.

### Key

One 32-byte key per channel:

- Argon2id, version 1.3 (`0x13`), memory 19456 KiB (19 MiB), 2 passes,
  1 lane, 32 bytes of output.
- Password: the passphrase's UTF-8 bytes, exactly as entered.
- Salt: the ASCII string `local-takkie/v1/channel/<n>`, with `<n>` the
  channel number in decimal (`1` to `10`).

The channel is in the salt, so the same passphrase gives a different key on
each channel.

### Sealed packets

On a private channel every packet (Audio, Hello and Bye) is sealed with
ChaCha20-Poly1305 (RFC 8439):

- Set bit 0 of `flags`, then write the header.
- Nonce (12 bytes): header bytes 8 to 19, that is `sender_id` then `seq`.
- Associated data: all 24 header bytes, as sent.
- The packet is the header, then the encrypted payload, then the 16-byte tag.

A sender must never send two packets with the same `sender_id` and `seq`
under one key. Pick `sender_id` from a cryptographic random generator, and
start a new one (restart) before `seq` wraps.

### Receiving

For a packet on the receiver's own channel:

| Packet | Receiver has the channel key | Receiver has none |
|---|---|---|
| Sealed | Open it. If the tag fails, drop it. If `seq` was seen before from this sender, drop it. Otherwise use it. | Drop it |
| Not sealed | Drop it | Use it |

- Replays: keep, per sender, the newest `seq` accepted and which of the 63
  before it were seen. Accept a `seq` once; drop anything 64 or more behind
  the newest. Only update this after the tag has been checked.
- A dropped packet on the receiver's own channel means the two sides don't
  share a passphrase. local-takkie shows that to the user.

For a packet on another channel, the receiver has no key that fits:

- Audio is dropped.
- A sealed Hello only says "this sender exists": take the sender id and
  channel from the header and the address from the datagram, and nothing
  from the payload. An unsealed Hello is used as usual.
- A sealed Bye is ignored. An unsealed Bye is ignored too if that sender has
  sent sealed packets in the last 30 seconds; such a peer is removed when it
  goes quiet or its mDNS record goes away.

### Test vectors

Keys (passphrase, channel, key in hex):

```text
"correct horse battery staple", 1:
3a5f7f18f2bccbb3ef7b818a6aab748633b2a6aace8f0fcd66c4a2b259f78840

"takkie", 10:
2a1fa1e92997e7d4e20a36fd63c0ca893b6b63577084d3f5cba5e75cc929cb05
```

A sealed packet. Key: 32 bytes of `07`. Header before sealing: Audio,
channel 3, end-of-transmission flag, sender `0102030405060708`, seq 42,
timestamp 960. Payload: the 18 ASCII bytes `takkie test vector`.

```text
544b01010303000001020304050607080000002a000003c0
81a3a25cdf1c770c1831062793ebfc6c2ca484ed427c518c4325481c0974a3bc8879
```

The first line is the header as sent (flags `03`: encrypted and end of
transmission), the second the encrypted payload and the tag. These are the
vectors the tests in `key.rs` and `seal.rs` use, and they were checked
against the reference Argon2 library and OpenSSL.

## Versioning

- The `version` byte changes for anything an older receiver would get wrong:
  new kinds, new header fields, a different layout.
- New flags can be added without a new version, because receivers ignore
  flag bits they don't know.
- The reserved bytes must stay zero in version 1, so a later version can use
  them.

## Examples

An Audio packet header: channel 3, no flags, sender `0102030405060708`, seq
`0A0B0C0D`, timestamp `11223344`.

```text
54 4B 01 01 03 00 00 00 01 02 03 04 05 06 07 08 0A 0B 0C 0D 11 22 33 44
```

A Hello header: channel 10, sender and seq all ones, timestamp 0.

```text
54 4B 01 02 0A 00 00 00 FF FF FF FF FF FF FF FF FF FF FF FF 00 00 00 00
```

Both are checked against the encoder by the `spec_examples_match_the_encoder`
test.
