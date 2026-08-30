# Architecture

```text
audio device / Android AudioRecord
            |
            v
consumer-owned capture and resampling
            |
            v
qsonaut-third-party adapters  ---> protocol libraries and their licenses
            |
            v
qsonaut-modems (this repository) ---> normalized events and timing contracts
            |
            v
consumer-owned UI, radio state, TX safety, QSO automation, and logging
```

The canonical decoder boundary is mono floating-point PCM with an explicit
sample rate. Consumers negotiate native device rates at capture time and
resample to the rate required by a selected adapter. No adapter may assume a
desktop audio API or an Android lifecycle.

`SlotGate` is a reusable timing primitive, not a scheduler. A consumer remains
responsible for clock selection, buffering, TX-slot suppression, cancellation,
and worker ownership.

The distinction between a full-rate capture stream and a modem-specific
decoder stream is normative. See
[AUDIO-BOUNDARY.md](AUDIO-BOUNDARY.md) before adding rate conversion or slot
assembly to a consumer.
