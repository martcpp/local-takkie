# How the voice is processed

What happens to sound between your microphone and someone else's speaker,
and which settings change it. The reasons behind the choices are in the
decision records: [0012](adr/0012-rnnoise-for-noise-suppression.md) to
[0016](adr/0016-our-own-voice-processing-on-android.md).

Everything runs on 48 kHz mono, in 20 ms frames.

## Sending

Each frame from the microphone goes through these stages, in this order.
The code is `VoiceChain` in `crates/takkie-engine/src/audio/voice.rs`; the
stages without dependencies are in `crates/takkie-core`.

| # | Stage | What it does | Setting |
|---|---|---|---|
| 1 | Low cut | Removes everything below 100 Hz: rumble, handling noise, mains hum. A voice has nothing there | Always on |
| 2 | Noise removal | RNNoise pushes down what isn't a voice. Normal keeps 15% of the microphone so it sounds natural; strong keeps none. Adds 10 ms of delay | Off, normal (default), strong |
| 3 | Level meter | Measured here, so the meter shows your microphone, not the evened-out result | |
| 4 | Even volume | Brings a quiet or loud voice to about -20 dB. It only listens while you speak, so pauses aren't turned up. At most +30 dB, at least -15 dB | On (default), off |
| 5 | Clarity | Lifts the band around 3 kHz, where consonants are, and thins everything below 250 Hz. Normal: +4 and -3 dB. Strong: +7 and -5 dB. A soft limiter follows | Off, normal (default), strong |
| 6 | Opus | 32 kbit/s, voice mode, with error correction for 10% loss. 32 is the lowest rate at which Opus sends the whole voice (to 20 kHz); at 24 it sends half | Fixed |

Stages 2, 4 and 5 only run on frames that are sent. While you aren't
talking, the microphone only feeds the meter, which costs almost nothing.

Change the settings with `N`, `L` and `C` in the terminal app, or the three
buttons under Sound in the desktop and phone app. They are remembered.

### Microphone warnings

- **Too loud:** the microphone hit full scale. Nothing after that point can
  repair it. Move back, or turn the input down in your system's settings.
- **Too quiet:** even volume has used all of its +30 dB. Move closer.

Each warning stays for two seconds after its cause.

## Receiving

Per talker, in the mixer (`crates/takkie-engine/src/audio/mix.rs`):

| # | Stage | What it does |
|---|---|---|
| 1 | Jitter buffer | Holds packets about 60 ms to put them in order |
| 2 | Opus decoder | A lost packet is rebuilt from the next one's error correction, or guessed when that is lost too |
| 3 | Even voices | The same leveller as on the sending side. It does nothing to a sender that already evens its own voice; it is for people on an older version |
| 4 | Mix | All talkers are added together |
| 5 | Volume | 0 to 200% |
| 6 | Beeps and limiter | The limiter keeps two loud talkers, or 200%, inside full scale |

## Getting a clear voice in a loud room

In rough order of how much each helps:

1. **Get the microphone close.** Every doubling of the distance loses 6 dB
   of voice while the room stays as loud. A headset, or the phone held like
   a handset, beats any setting. In the tests nothing rescued a talker who
   was far from the microphone.
2. **Leave noise removal on.** Try strong if the room is very loud.
3. **Leave even volume on,** so nobody has to reach for the volume between
   talkers.
4. **Use clarity** when the listener is in noise too. Strong is for a small
   speaker in a loud room; it can sound sharp on good headphones.
5. **Listeners in the same room as the talker should wear headphones,** or
   mute that person. Otherwise they hear the voice twice. See
   Troubleshooting in the README.

## What isn't there

- **Echo cancellation.** Half-duplex (your speaker is muted while you talk)
  does that job ([ADR 0015](adr/0015-no-echo-cancellation-for-now.md)).
- **The phone's own voice processing.** Ours is the same on every device
  ([ADR 0016](adr/0016-our-own-voice-processing-on-android.md)).
- **A stronger noise remover.** DeepFilterNet scored better in very loud
  rooms but is heavy and unmaintained
  ([ADR 0013](adr/0013-deepfilternet-not-yet.md)).
- **Neural loss hiding in Opus.** It helps when several packets in a row are
  lost; waiting for numbers from a real network
  ([ADR 0014](adr/0014-deep-plc-after-field-data.md)).

## Trying a change

The voice lab runs a recording through the sending stages and Opus and
writes what the far end would hear, with levels before and after. See
"Listening to a change in the voice" in
[CONTRIBUTING.md](../CONTRIBUTING.md).

## How far to trust the numbers

The settings were chosen by measuring, not by listening: synthesised speech,
a synthetic crowd and reverberation, and a simplified intelligibility score.
They have not yet been judged by ear in a real room. The defaults are to be
confirmed, or changed, by the field test.
