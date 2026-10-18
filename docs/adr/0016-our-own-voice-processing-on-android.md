# 0016. On Android, our own voice processing, not the phone's

- **Status:** Accepted
- **Date:** 2026-10-13

## Context

Android phones have their own echo canceller, noise suppressor and gain
control. They only run when the microphone stream is opened with the
VOICE_COMMUNICATION input preset (and, for echo cancellation, the playback
stream with the matching usage). E20.11 asked whether Takkie should use
them.

**How we would get there**

- cpal 0.18 opens AAudio streams with no way to set the input preset, the
  usage or a session id. The `ndk` crate underneath has safe methods for all
  three; cpal just doesn't call them.
- Upstream knows: cpal PR #995 adds the input preset and is still open
  (since July 2025). The maintainer wants it to land through a general
  extension API (issue #1334) and has dropped the earlier attempt (#1010).
  No release is planned for it.
- So today it means a patched cpal, or opening the AAudio stream ourselves
  next to cpal. Both are code we would have to keep in step with cpal.

**What it would cost**

- Phones differ. Reports for this preset include microphones recorded at
  16 kHz instead of 48, distorted sound on some devices, and the loss of the
  low-latency path we asked for in E6.10 on most.
- The sound would differ from phone to phone and from the desktop, and our
  own switches (noise removal, even volume, clarity) would stack on top of
  processing we cannot see or turn off.

**What we have now.** Since E20, noise suppression, levelling and clarity
are ours and the same on every platform, and half-duplex removes the need
for echo cancellation (ADR 0015).

Not done: the comparison on the test phone that the ticket asked for
(quality, the sample rate actually given, latency path, CPU). It needs the
phone, a patched build and someone listening.

## Decision

Use our own processing on Android. Do not patch cpal or open AAudio streams
ourselves for the voice preset.

## Consequences

- The same voice on every device, tuned in one place.
- Our noise suppression runs on the phone's CPU rather than in its audio
  chip. What that costs on a budget phone is still to be measured (E20.14).
- No echo cancellation on the phone. Half-duplex covers it.

## Reconsider if

- cpal ships its extension API with the Android input preset. Then trying
  the phone's processing is a few lines, and worth an afternoon with the
  test phone.
- The field test shows our noise suppression costs too much battery or CPU
  on the phone.
- We decide to offer full duplex on phones (ADR 0015): the phone's echo
  canceller knows its own speaker and microphone better than a general one.

Alternatives considered: a patched cpal with the preset behind a setting
(cheap to write, but a fork to maintain and a second sound to support);
letting the user choose (two code paths, and nobody can say which is better
on their phone without trying both).
