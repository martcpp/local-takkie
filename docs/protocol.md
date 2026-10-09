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

A peer that sends nothing for 10 seconds is treated as gone.

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

Written in E11. Until then, the encrypted flag is never set.

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
