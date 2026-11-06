# 0013. DeepFilterNet: better in loud rooms, not adopted yet

- **Status:** Accepted
- **Date:** 2026-10-13

## Context

RNNoise (ADR 0012) is light, but it is known to be weaker on crowds and on
reverberation. DeepFilterNet 3 is the strongest open model that still runs in
real time, so E20.13 asked whether it is worth offering as a harder setting.

It was built in a scratch project and run on the same clips as ADR 0012.
"Intelligibility" is the same simplified STOI; "noise" is the level in the
talker's pauses. Times are for 10 ms of sound on the development laptop.

| Clip | Processor | Intelligibility | Noise |
|---|---|---|---|
| Crowd, 10 dB quieter than the voice | none | 0.921 | -33.0 dB |
| | RNNoise, 85% blend (our normal) | 0.951 | -46.8 dB |
| | DeepFilterNet 3 | 0.950 | -62.8 dB |
| Crowd, 5 dB quieter | none | 0.869 | -28.0 dB |
| | RNNoise, 85% blend | 0.913 | -41.0 dB |
| | DeepFilterNet 3 | 0.929 | -54.1 dB |
| Crowd as loud as the voice | none | 0.791 | -23.0 dB |
| | RNNoise, 85% blend | 0.839 | -35.3 dB |
| | DeepFilterNet 3 | 0.877 | -48.7 dB |
| Talker far from the microphone, crowd | none | 0.718 | -26.7 dB |
| | RNNoise, 85% blend | 0.734 | -31.3 dB |
| | DeepFilterNet 3 | 0.784 | -42.2 dB |

| | RNNoise | DeepFilterNet 3 |
|---|---|---|
| Time per 10 ms, average | 0.15 to 0.3 ms | 1.3 to 2.0 ms |
| Time per 10 ms, worst seen | under 1 ms | 22 ms |
| Added delay | 10 ms | 30 ms |
| Size | small | model 8 MB; the test program was 24 MB |
| Where it comes from | crates.io, maintained | only its Git repository; last commit 2024-09-25 |

What the numbers say:

- In mild noise the two are equally clear; DeepFilterNet only leaves less
  noise behind.
- In loud noise, and for a talker far from the microphone, DeepFilterNet is
  clearly better. It is the only thing tried so far that helps the far
  talker, the case ADR 0012 called a matter of holding the microphone close.
- It costs 7 to 10 times the CPU, and one block took 22 ms, more than two
  blocks' worth of time. On a budget phone that is likely to stutter. It was
  not built for Android in this spike.
- The real-time code is not on crates.io (the published `deep_filter` 0.2.5
  is an older crate without it). The repository does not compile against the
  current `tract`; it needed its own two-year-old lock file. Shipping it
  means a Git dependency and versions we would have to hold back ourselves.

## Decision

Do not add it now. RNNoise stays the only noise suppressor.

## Consequences

- The far-talker and very-loud-room cases stay weaker than they could be.
- No Git dependency, no pinned-back inference library, no 8 MB model in the
  phone app.

## Reconsider if

- The field test in the church (E20.14) finds that voices are still hard to
  understand with noise removal on strong. Then the next step is a
  desktop-only "maximum" setting, built behind a Cargo feature, and a
  measurement on the phone before offering it there.
- DeepFilterNet's real-time crate is published and maintained again, or a
  comparable model appears on crates.io.

Alternatives considered: adding it for desktops only right away (the
maintenance cost is the same, and nobody has heard yet whether RNNoise falls
short in the real room).
