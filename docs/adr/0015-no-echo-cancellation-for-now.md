# 0015. Echo cancellation is possible in pure Rust, and still not built

- **Status:** Accepted
- **Date:** 2026-10-13

## Context

ADR 0009 made half-duplex the default because we had no echo cancellation,
and named the alternative: full duplex with WebRTC's audio processing, "a C++
build". Since then `sonora` has ported that code, including the AEC3 echo
canceller, to pure Rust (BSD-3-Clause). E20.10 asked whether that makes
full duplex on a loudspeaker worth offering.

AEC3 was run in a scratch program on a simulated echo: a far voice played by
a speaker, picked up by the microphone, with the near talker silent for the
first half and speaking in the second. "Echo removed" is measured while only
the far side talks. "Near voice" is the simplified STOI of ADR 0012 for the
near talker while both talk.

| Echo path | Echo removed | Near voice, no canceller | Near voice, with it |
|---|---|---|---|
| Direct, 20 ms, echo as loud as the voice | 42 dB | 0.866 | 0.795 |
| Direct, 80 ms | 45 dB | 0.852 | 0.784 |
| Direct, 80 ms, delay not given to it | 44 dB | 0.852 | 0.799 |
| Direct, 400 ms, delay not given | 45 dB | 0.850 | 0.769 |
| Direct, speaker and microphone clocks 0.05% apart | 30 dB | 0.850 | 0.752 |
| Direct, clocks 0.5% apart | 5 dB | 0.858 | 0.772 |
| Direct, distorting speaker | 40 dB | 0.847 | 0.727 |
| Mostly reverberation (a hard room) | 11 to 15 dB | 0.855 | 0.749 |

Time: 0.25 to 0.45 ms per 10 ms on the development laptop, more than noise
suppression.

What this says:

- It works where it is meant to: a direct path from speaker to microphone
  loses over 40 dB, and it finds the delay by itself.
- Clocks are the risk. The speaker and the microphone are separate devices
  with separate clocks in our engine. A small difference is tolerated; a
  large one defeats it. Real sound cards sit somewhere between the two rows.
- A reverberant room defeats most of it: 11 to 15 dB is an echo you still
  hear. A church is that room.
- While both sides talk, the near voice scores lower with the canceller than
  without. That is how cancellers behave: they duck the microphone to be
  safe. Half-duplex has no such cost, because nobody talks over anybody.

This was a simulation. It was not tried on a real laptop or the test phone
at full volume, which the ticket asked for; that needs someone in the room.

## Decision

Stay half-duplex only. Do not build echo cancellation now.

The app is push-to-talk: one person talks at a time by design, and
half-duplex already makes a loudspeaker safe. Full duplex would add a
reference signal between the playback and capture threads, drift handling,
more CPU on the phone, and a result that is weakest in exactly the room this
epic is about.

## Consequences

- `half_duplex = false` stays a setting for people with headphones. The
  terminal app now says so at start-up (E20.9).
- ADR 0009's alternative is no longer "a C++ build". If we do want full
  duplex later, the code is a crate away.

## Reconsider if

- People ask for phone-call style conversation, as ADR 0009 says.
- We want to detect a peer in the same room automatically (E20.9 left that
  out): the canceller's delay estimator is most of what that needs.

Alternatives considered: the phone's own echo canceller (the Android spike,
E20.11); building it now behind a setting (the simulation gives no reason to
expect it to work well in a reverberant room, and nobody has asked for full
duplex).
