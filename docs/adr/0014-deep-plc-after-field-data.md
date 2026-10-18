# 0014. Opus's neural loss hiding: worth it for bursts, decided after field data

- **Status:** Accepted
- **Date:** 2026-10-13

## Context

When a packet is lost we rebuild it from the FEC data in the next packet, or,
when that is lost too, let Opus guess (packet loss concealment, PLC). Since
1.5, libopus can guess with a small neural network ("Deep PLC"), switched on
by building it in and setting the decoder's complexity to 5 or more. We
already build libopus 1.6.1, without it. E20.12 asked what turning it on
takes and what it buys.

**What it takes**

- A build feature of `opusic-sys` (`dred`, which brings Deep PLC; `osce` for
  the speech enhancement that goes with it). The program grows by a few MB
  of network weights.
- A decoder complexity setting. The `opus` 0.4 crate we use does not expose
  it. `opusic-c` 1.6 does: a safe binding by the author of `opusic-sys`, the
  same C library underneath, BSD-3-Clause. So the way in is to change
  binding, not to write `unsafe` ourselves.

**What it buys.** The clean test talker at 32 kbit/s with FEC, packets
dropped four ways, decoded by the same build with the classic PLC
(complexity 0) and the neural one (complexity 10). "Intelligibility" is the
simplified STOI of ADR 0012.

| Loss | Classic | Neural |
|---|---|---|
| 10%, single packets | 0.989 | 0.988 |
| 20%, single packets | 0.969 | 0.973 |
| 16% in bursts of 3 packets | 0.909 | 0.928 |
| 18% in bursts of 5 packets | 0.881 | 0.910 |

| | Classic | Neural |
|---|---|---|
| Decoder time per 20 ms frame, laptop | 0.05 ms | 0.22 ms |
| Worst frame seen | 0.5 ms | 2.8 ms |

- Single lost packets: no difference. FEC already rebuilds them.
- Bursts, where FEC has nothing to work from: a clear gain, and Wi-Fi loses
  packets in bursts more than one at a time.
- The decoder costs four times as much, per talker. Still about 1% of a
  slow laptop core.

Not measured: the phone, the size on Android, and DRED (extra redundancy in
the packets, which would also need both ends on the new version).

## Decision

Not yet. First learn from the field test (E20.14) how packets are really lost
on the church's Wi-Fi: the app already counts frames recovered by FEC,
frames guessed, and frames left silent.

If bursts show up (frames guessed or silenced, not just recovered), move the
codec from `opus` to `opusic-c`, build with `dred`, and set the decoder
complexity on desktop and phone.

## Consequences

- Until then a burst of lost packets sounds as it does now.
- No change of binding and no bigger program for a gain we have not yet
  seen a need for.

## Reconsider if

- The field test shows guessed or silenced frames. That is the trigger.
- The `opus` crate gains a decoder complexity setting, which would make the
  change smaller.

Alternatives considered: turning it on now (changes the binding under the
whole audio path days before the first real test); DRED (not interoperable
with older versions, and the redundancy costs 12 to 32 kbit/s).
