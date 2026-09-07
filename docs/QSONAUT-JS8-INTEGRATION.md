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

For a live receiver worker, use `Js8RxSession` from the JS8 crate. It accepts
already-normalized 12 kHz chunks, keeps a bounded slot-sized buffer, and
performs a caller-selected number of candidate decodes per `poll()` call. It
does not own a capture device, clock, worker, cancellation token, slot policy,
or UI state:

```rust
use qsonaut_js8::{Js8RxConfig, Js8RxSession, Js8ScanConfig};

let mut receiver = Js8RxSession::new(Js8RxConfig::default(), Js8ScanConfig::default());
receiver.push_samples(&samples_12k_chunk)?;
for result in receiver.poll(2)? {
  // Map result.result.event and result.result.message into the UI.
  println!("{} at sample {}", result.result.event.message, result.candidate_sample);
}
```

`poll()` is intentionally bounded rather than an implicit background loop. The
consumer can run it on its modem worker, check cancellation between calls, and
decide when a slot is complete or should be reset.

For the primary waterfall-wide path, use `Js8RxSession::waterfall(config)` or
`Js8ScanConfig::waterfall()` with `Js8RxSession::new`. This searches a bounded
200–3000 Hz audio band, computes a shared FFT spectrum over the known Costas
symbols, retains eight ranked frequency hypotheses, and extracts up to four
signals per candidate window. The exact timing, drift, FEC, and CRC decoder is
only run for those ranked peaks; it does not brute-force Costas correlation at
every frequency bin. Callers can lower the bounded limits when worker CPU or
cancellation latency is more important than recall.

When `Js8ScanConfig::dedup_samples` is non-zero, the session also suppresses a
successful message that reappears within that sample distance across separate
`poll()` calls. This prevents overlapping rolling windows from reporting the
same transmission repeatedly. Call `reset()` when starting a new independent
slot or recording.

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
3. Build a shared Costas-symbol FFT spectrum and rank likely carrier peaks.
4. Refine only those candidates, then correlate aligned 8-FSK symbols into soft
  metrics.
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
For waterfall-wide operation, set
`Js8ScanConfig::waterfall_frequency_range_hz` to an absolute `(low_hz,
high_hz)` band, then raise the bounded `max_frequency_hypotheses` and
`max_signals_per_window` limits as needed. The scanner ranks Costas
spectral hypotheses, decodes the strongest CRC-valid signal, and subtracts it
before searching the residual for additional channels. Previously decoded
carriers are excluded from later residual passes so a weak cancellation cannot
starve the remaining channels. These limits remain consumer-visible so a
worker can budget CPU and cancellation opportunities.
For each candidate window, the scanner retains the configured number of
strongest distinct Costas frequency hypotheses (three by default) and lets
exact FEC/CRC validation reject weaker interferers that happen to produce the
strongest coarse peak.

## Current limitations

- The default RX adapter remains single-signal. The recording scanner has
  bounded waterfall-wide multi-signal extraction with shared FFT candidate
  acquisition when configured with an absolute frequency band and larger
  per-window limits; it is not a full JS8Call detector or scheduler.
- Frequency acquisition ranks a bounded grid using the shared Costas-symbol
  spectrum, followed by a quadratic track fitted
  across the three Costas blocks. Timing-aware demodulation performs a bounded
  global Costas search with fractional-sample interpolation for linear and
  quadratic sample-clock drift and exposes both estimates through
  `Js8RxResult`.
- Timing acquisition uses a coarse Costas-symbol subset for broad candidate
  rejection, followed by full-symbol refinement. Fractional-symbol correlation
  reuses the constant interpolation fraction within each symbol to reduce hot
  loop overhead without changing the final timing model.
- Successful decode events now include a decoder-derived relative SNR estimate
  (capped at 99 dB for ideal synthetic audio); it is suitable for ranking and
  consumer quality policy, not calibrated RF measurement.
- `Js8RxResult` also reports the fitted linear carrier drift in hertz per
  second and quadratic curvature in hertz per second squared, while
  `DecodeEvent.audio_frequency_hz` remains the frame-start carrier estimate.
  Its `timing_drift_samples_per_second` and
  `timing_curvature_samples_per_second2` fields report the corresponding
  sample-clock displacement fit.
- The decoder has verified clean-frame and single-hard-bit behavior, but is not
  yet a JS8Call-equivalent noisy-channel performance implementation.
- Soft-bit generation now follows the JS8Call reference strategy: strongest
  tone per bit half, separate normalized amplitude and log metrics, and
  oracle-style erasure retries through the LDPC decoder. This is an isolated
  interoperability improvement, not a complete noisy-channel match.
- Failed primary decodes receive one additional bounded retry using a
  per-symbol lower-floor estimate for soft metrics. This preserves the normal
  decode path while providing a fallback for locally varying interference; it
  has not yet recovered the difficult `A_2_1.wav` oracle fixture.
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
  metadata and the directed-command table. Legacy Huffman data payloads can
  now be decoded through `decode_legacy_huffman_data`; dense JSC compressed
  payloads remain losslessly preserved as encoded data, and long-message
  fragments can be joined with the bounded `Js8MessageReassembler` when the
  consumer supplies JS8Call's explicit `First`/`Last`/`Data` transmission
  flags. The reassembler does not infer those flags from payloads or own
  consumer persistence.
- The optional JS8Call media scan is a capability measurement only. The
  current bounded scan uses decimated Costas acquisition followed by exact
  timing/frequency refinement. A partial release run after baseline subtraction
  decoded six of the first seven processed recordings, including
  `A_1_4.wav`, `A_2_3.wav`, `A_2_5.wav`, `A_2_6.wav`, `A_2_9.wav`, and
  `A_3_3.wav`; the complete 9-recording recall has not yet been finalized.
  This is not full noisy-channel interoperability; drift tracking,
  multi-signal subtraction, stronger FEC behavior, and the oracle's overlapping
  spectrum pipeline remain modem work. The scanner now performs one bounded
  residual-cancellation retry after a valid decode, persists that cancellation
  across later candidate windows, and can recover a weaker overlapping
  synthetic frame. It is not an oracle-equivalent subtraction pipeline.
  Duplicate candidates are ranked by
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
- Duplicate suppression, CQ/heartbeat semantics, directed commands, and
  application events remain consumer/message layers. Signal subtraction is
  owned by the modem, but the current implementation is limited to two
  signals per candidate window and does not claim full JS8Call parity.
- The current public adapter is aligned to one frame/window, not a complete
  multi-signal 60-second JS8Call receiver.
- `Js8RxSession` provides bounded chunk ingestion and candidate polling for a
  consumer-owned worker. Successful bounded signal cancellation is retained in
  its rolling buffer across polls. It is not a clock or slot scheduler;
  QSONaut still owns capture timing, `SlotGate`, cancellation, and lifecycle
  decisions.

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
