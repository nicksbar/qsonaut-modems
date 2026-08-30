# Consumer integration plan

This is the planned migration sequence; no consumer is modified by this
repository's initial implementation.

1. Validate the contracts and adapters with deterministic generated and golden
   audio fixtures.
2. Add a QSONaut feature branch that replaces only the pure decode calls in
   `workers/decode.rs`, retaining existing GUI state and safety behavior.
3. Compare old and new decode results, timing, frequency, SNR, and no-decode
   behavior before removing old dependencies.
4. Add the same adapter to QSONoid's Rust engine. Kotlin remains responsible
   for Android audio lifecycle and route selection; Rust receives audio blocks
   and returns normalized events.
5. Keep slot policy, TX disarm, QSO sequencing, logging, PSK reporting, and
   presentation in each consumer until shared requirements are demonstrated.

## Audio integration rule

Do not reinterpret the station's full-rate capture as a narrowband decoder
block. Fan out the capture stream first, then create the adapter-specific
`AudioBlock` with a stateful anti-aliased resampler. For the current WSJT
adapters, that block is 12 kHz mono; the capture stream may remain 48 kHz for
the waterfall, monitor, recording, or other consumers. See
[AUDIO-BOUNDARY.md](AUDIO-BOUNDARY.md).
