# Audio boundary and rate ownership

The shared contract deliberately separates the station's capture stream from
the audio representation required by an individual modem.

```text
radio / Android AudioRecord
        |
        +--> full-rate monitor, waterfall, recording, and other consumers
        |
        +--> stateful anti-aliased resampler --> modem-specific AudioBlock
```

`AudioBlock` carries an explicit sample rate and contains mono `f32` samples.
It is not a declaration that every consumer must capture at that rate, and it
does not own conversion, device I/O, or buffering policy.

## Typical WSJT path

The current QSONaut/QSONoid station boundary is 48 kHz mono PCM. The pinned
WSJT-family adapters in `qsonaut-third-party` consume a separate 12 kHz mono
representation because that is the input convention used by `mfsk-core`.
The 48 kHz stream must remain available for the waterfall, monitor, recording,
and any future decoder that needs a different rate.

Twelve kHz here means sample rate, not twelve kHz of occupied radio
bandwidth. An FT8 decoder window is normally 15 seconds, or 180,000 samples
at 12 kHz, with the adapter's frequency search range selecting the useful
part of that spectrum.

## Resampling requirements

Consumers that convert a live stream must:

- use a stateful, anti-aliased resampler across callback/chunk boundaries;
- preserve timestamps and report gaps or discontinuities;
- keep slot assembly and decode scheduling separate from rate conversion;
- avoid silently dropping the full-rate stream after creating a decoder view;
- validate the resulting `AudioBlock` rate before calling an adapter.

A stateless sample average or taking every fourth sample is not an accepted
live-audio implementation. It may be useful as a deliberately labelled test
fixture, but it is not a substitute for the consumer's production resampler.

This repository does not provide a universal 48 kHz-to-12 kHz policy. That
policy belongs at the consumer/audio boundary until a separately reviewed,
cross-platform streaming resampler is extracted.
