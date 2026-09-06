# QSONaut JS8 integration

`qsonaut-js8` is the first-party GPL-3.0-or-later JS8 modem crate in this
repository. It owns JS8 framing, waveform generation, synchronization, FEC,
CRC validation, and conversion to the generic modem contracts. QSONaut owns
capture, resampling policy, clocks, slot gating, cancellation, PTT safety, UI,
and message workflow.

## Crate dependency

From a sibling consumer during local development:

```toml
qsonaut-modems = { path = "../qsonaut-modems/crates/qsonaut-modems" }
qsonaut-js8 = { path = "../qsonaut-modems/crates/js8" }
```

The JS8 crate already depends on the generic contracts crate and exposes the
adapter APIs below. QSONaut does not need a protocol-specific dependency in
`qsonaut-modems` itself.

## TX path

Build a validated 12 kHz mono block. The returned samples are normalized
`f32` audio in `[-1, 1]`; the adapter does not own a sound device or transmit
anything.

```rust
use qsonaut_js8::{encode_audio_block, Js8Mode, Js8TxConfig};

let config = Js8TxConfig {
    mode: Js8Mode::Normal,
    base_frequency_hz: 1500.0,
    frame_type: 0,
};
let block = encode_audio_block("0123456789AB", config)?;
// block.sample_rate_hz == 12_000
// block.samples is ready for the consumer's TX audio pipeline.
```

The consumer must apply its own amplitude policy, output-device conversion,
PTT sequencing, global disarm, and late-worker suppression. Do not call this
function from a callback that owns radio safety unless the consumer has already
made the TX decision.

## RX path

The adapter accepts a mono `AudioBlock` at exactly 12 kHz. The simplest input
is one complete frame. A rolling slot window may contain up to one symbol of
leading uncertainty; the adapter searches the Costas arrays over that range.

```rust
use qsonaut_js8::{decode_audio_block, Js8Mode, Js8RxConfig};

let config = Js8RxConfig {
    mode: Js8Mode::Normal,
    center_frequency_hz: 1500.0,
    frequency_half_width_hz: 5.0,
    frequency_step_hz: 0.5,
    max_fec_iterations: 10,
};
let batch = decode_audio_block(&audio_block, config)?;
for event in batch.events {
    println!("{} at {:?} Hz", event.message, event.audio_frequency_hz);
}
```

For JS8Call-level message interpretation, use the detailed result. It retains
the validated physical frame and exposes a typed `Js8Message` for heartbeat,
compound, directed, and data frames:

```rust
use qsonaut_js8::{decode_audio_block_detailed, Js8Message};

let result = decode_audio_block_detailed(&audio_block, config)?;
match result.message {
  Js8Message::Directed { from, to, command } => {
    println!("{from} -> {to}: {}", command.name);
  }
  Js8Message::Heartbeat { callsign, grid, .. } => {
    println!("heartbeat from {callsign} at {grid:?}");
  }
  other => println!("{other:?}"),
}
```

Typed semantic TX is available through `encode_message_audio_block`; it builds
heartbeat, compound, directed, and raw/data frames into the same validated
12 kHz audio boundary. The consumer still owns callsign policy, scheduling,
PTT safety, and application replies.

The current RX sequence is:

1. Validate 12 kHz and finite samples.
2. Find a Costas-based frame boundary when the window is larger than one frame.
3. Search the configured absolute audio-frequency grid.
4. Correlate all aligned 8-FSK symbols into soft metrics.
5. Convert tone metrics to bit LLRs.
6. Run bounded `(174,87)` min-sum LDPC decoding.
7. Reconstruct the 12-character payload, extract frame type, and validate CRC.
8. Return a generic `DecodeBatch` with `ModemId("js8")`, message, frequency,
   and measured leading-window offset.

`snr_db` is a decoder-derived relative quality estimate on successful decodes;
it is not a calibrated RF noise-floor measurement. `delta_time_seconds`
describes the recovered leading offset, not a network or radio clock
measurement.

## Audio boundary

If capture is 48 kHz, keep the capture stream in the consumer and use the
shared stateful normalizer before calling JS8:

```rust
use qsonaut_modems::AudioNormalizer;

let mut normalizer = AudioNormalizer::new(48_000)?;
let samples_12k = normalizer.process_f32_mono(&capture_chunk)?;
```

Accumulate normalized samples in a consumer-owned rolling buffer. Do not call
`decode_audio_block` on arbitrary capture chunks; provide a complete frame or a
window with enough samples for the one-symbol timing search.

## Slot timing

Use `slot_spec(mode)` to obtain generic duration metadata:

```rust
use qsonaut_js8::{slot_spec, Js8Mode};
use qsonaut_modems::SlotGate;

let spec = slot_spec(Js8Mode::Normal);
let mut gate = SlotGate::default();
// The consumer owns the clock and decides when buffer_ready is true.
if gate.observe(slot_number, position, spec, buffer_ready) {
    // Extract a complete JS8 window and invoke decode_audio_block once.
}
```

The adapter does not create workers, schedule decode attempts, suppress TX
inside RX slots, or decide what to do after a decode failure.

## Recording scan

For a complete recording, use the bounded scanner and choose the candidate
spacing in the consumer. It returns only CRC-valid frames, reports coarse
Costas quality on each detailed result, and suppresses duplicates within the
configured sample distance by retaining the stronger candidate:

```rust
use qsonaut_js8::{scan_audio_block, Js8ScanConfig};

let batch = scan_audio_block(
  &recording,
  config,
  Js8ScanConfig {
    step_samples: 6_000, // 500 ms at 12 kHz
    max_candidates: 120,
    ..Js8ScanConfig::default()
  },
)?;
```

The scanner is deliberately not a JS8Call scheduler or 60-second detector:
the consumer still supplies recording windows, cancellation, slot policy, and
thread ownership. Candidate spacing is a performance/recall trade-off; a
smaller step improves acquisition of unknown signal starts but costs more CPU.

## Current limitations

- RX is single-signal only.
- Frequency acquisition is a bounded grid search with a quadratic track fitted
  across the three Costas blocks. Timing-aware demodulation now performs a
  bounded global Costas search with fractional-sample interpolation for linear
  sample-clock drift and exposes the estimate through `Js8RxResult`; higher-
  order clock behavior remains future work.
- Successful decode events now include a decoder-derived relative SNR estimate
  (capped at 99 dB for ideal synthetic audio); it is suitable for ranking and
  consumer quality policy, not calibrated RF measurement.
- `Js8RxResult` also reports the fitted linear carrier drift in hertz per
  second and quadratic curvature in hertz per second squared, while
  `DecodeEvent.audio_frequency_hz` remains the frame-start carrier estimate.
- The decoder has verified clean-frame and single-hard-bit behavior, but is not
  yet a JS8Call-equivalent noisy-channel performance implementation.
- Soft-bit generation now follows the JS8Call reference strategy: strongest
  tone per bit half, separate normalized amplitude and log metrics, and
  oracle-style erasure retries through the LDPC decoder. This is an isolated
  interoperability improvement, not a complete noisy-channel match.
- Coarse Costas acquisition now scores known tones against the average of the
  seven rejected tones, approximating JS8Call's baseline-resistant `t/t0`
  sync ratio, aggregated per Costas block. It passes deterministic common-floor
  coverage; frame-wide lower-envelope baseline subtraction recovered
  `A_1_4.wav` as `Vk4xfHSNwzaX` (frame type 3).
- Soft metrics also sample guard tones immediately outside the 8-FSK band when
  estimating the lower noise envelope. This passed deterministic coverage but
  did not recover `A_2_1.wav`; the remaining gap is not solved by scalar
  in-band or guard-tone floor estimation alone.
- The message layer now decodes and encodes the compact JS8Call heartbeat,
  compound, compound-directed, and directed layouts, including callsign/grid
  metadata and the directed-command table. Legacy compressed data payloads are
  preserved as encoded data; Huffman/JSC decompression and long-message
  reassembly remain consumer/protocol work.
- The optional JS8Call media scan is a capability measurement only. The
  current bounded scan uses decimated Costas acquisition followed by exact
  timing/frequency refinement. A partial release run after baseline subtraction
  decoded six of the first seven processed recordings, including
  `A_1_4.wav`, `A_2_3.wav`, `A_2_5.wav`, `A_2_6.wav`, `A_2_9.wav`, and
  `A_3_3.wav`; the complete 9-recording recall has not yet been finalized.
  This is not full noisy-channel interoperability; drift tracking,
  multi-signal subtraction, stronger FEC behavior, and the oracle's overlapping
  spectrum pipeline remain modem work. Duplicate candidates are ranked by
  coarse sync quality and then decoder-derived SNR without changing
  chronological result ordering.
- Corpus scans can be made much faster for smoke checks with the optional
 `JS8CALL_SCAN_FREQUENCY_HALF_WIDTH` and `JS8CALL_SCAN_FREQUENCY_STEP`
 environment variables, in addition to `JS8CALL_SCAN_STEP`,
 `JS8CALL_SCAN_MAX_CANDIDATES`, `JS8CALL_MEDIA_FILE`,
 `JS8CALL_SCAN_START_SECONDS`, and `JS8CALL_SCAN_DURATION_SECONDS`. The last
 two slice the recording before scanning, providing a safe fast-forward effect
 over known-empty audio. Tests are CPU-bound rather than realtime; resampling
 or time-compressing the waveform would change JS8 symbol timing and is not a
 valid acceleration. A representative
 coarse profile reduced one fixture from about 257 seconds to about 6 seconds,
 but missed the weak `A_1_4.wav` signal; use the original dense settings for
 recall measurements.
- Tone correlation uses a phase-oscillator recurrence rather than evaluating
  sine and cosine for every sample. This accelerates both ordinary decoder
  tests and corpus scans while preserving the sample-rate and symbol timing
  model; the local JS8 debug suite dropped from about 55 seconds to about 19
  seconds.
- At the established full-fidelity settings, the optimized release scan of
  `A_2_1.wav` completed in 176.60 seconds but still produced no decode.
- A denser 500 ms candidate scan and scalar Costas refinements were tested
  before baseline subtraction and produced no additional decodes. Improving
  the remaining recall requires deeper oracle-style spectrum/baseline,
  synchronization, and FEC interoperability work; it is not solved by
  candidate spacing or scalar Costas weighting alone.
  Baseline subtraction is the first change to materially improve recall, but
  the final corpus total remains pending.
- Duplicate suppression, subtraction, CQ/heartbeat semantics, directed
  commands, and application events remain consumer/message layers.
- The current public adapter is aligned to one frame/window, not a complete
  multi-signal 60-second JS8Call receiver.

Do not claim full JS8Call interoperability until oracle-generated audio,
frequency/time offset sweeps, deterministic noise fixtures, and no-decode
fixtures pass in both directions.

## Migration checklist

- [ ] Add `qsonaut-js8` as a GPL-compatible dependency in the consumer.
- [ ] Preserve the existing capture and 48 kHz monitor/waterfall path.
- [ ] Fan out capture through a stateful 48 kHz→12 kHz normalizer.
- [ ] Keep `SlotGate`, rolling buffers, cancellation, and worker ownership in
      the consumer.
- [ ] Replace only the pure JS8 decode call with `decode_audio_block` initially.
- [ ] Map `DecodeBatch.events` into the existing UI/QSO flow.
- [ ] Route TX text through `encode_audio_block` only after consumer TX safety
      approves the operation.
- [ ] Compare old/oracle and new results before removing existing JS8 behavior.
- [ ] Add desktop and Android validation in the consumer repositories.
