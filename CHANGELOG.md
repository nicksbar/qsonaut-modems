# Changelog

All notable changes to `qsonaut-modems` are documented here.

## [Unreleased]

## [0.1.1] - 2026-09-21

### Added

- Added consumer-neutral deterministic modem fixture metadata, provenance, and
  normalized decode validation contracts for adapters and applications.

### Added

- Added explicit JS8Call and WSJT-X attribution, provenance, licensing-boundary,
  and non-bundled-source notices. The Rust implementation takes no credit for
  the original projects and identifies the upstream compatibility material it
  reproduces.
- Added the first-party `qsonaut-js8` crate under `crates/js8`.
- Added a bounded `Js8RxSession` for chunk-fed 12 kHz RX buffering and
  cancellation-friendly candidate polling without taking ownership of capture,
  clocks, workers, slot policy, or UI state.
- Added cross-poll duplicate suppression to `Js8RxSession` for overlapping
  rolling candidate windows, with reset-scoped result history.
- Added JS8 TX/RX support for Normal, Fast, Turbo, Slow, and Ultra modes.
- Added current JS8Call operator metadata for all five enabled speeds,
  including JS8 40/JS8 60 display names, slot periods, start delays,
  bandwidths, decoder thresholds, and heartbeat-network eligibility.
- Added batch and chunk-fed multi-speed receive APIs. Each selected speed keeps
  its own bounded streaming session so short modes can report without waiting
  for a complete Slow-mode window.
- Added directed-message packing and unpacking for the mentor's complete
  reserved-call/group map, including `@ALLCALL`, `@HB`, and activity groups.
- Aligned compact directed callsign packing with the mentor's six-position
  shape, portable suffix, and 3DA0/3X aliases; malformed shapes now return an
  error instead of reaching invalid arithmetic.
- Added public directed-command capability metadata for all 32 wire codes,
  including mentor autoreply, buffered-payload, checksum, and SNR sets without
  assigning transmission or persistence policy to the modem.
- Added current-mentor tone vectors generated directly from JS8Call Improved
  revision `e8a6121d859ba3b678b3485e7a14ed07df1bbee4` and verified them across
  all five enabled modes.
- Added an unequal-power multi-speed regression that recovers overlapping Fast
  and JS8 60 passbands with the stronger signal approximately 3.4 times the
  weak signal amplitude.
- Added JS8 alphabet packing, CRC-12 validation, `(174,87)` LDPC decoding,
  Costas synchronization, bounded recording scanning, and generic modem adapter
  integration through `qsonaut-modems` audio/events contracts.
- Added optional JS8Call WAV-corpus validation using the external oracle media
  fixtures.
- Added decoder-derived relative SNR reporting on successful decode events.
- Added deterministic linear and quadratic carrier-drift synthesis fixtures.
- Added three-Costas-block carrier tracking with fitted frequency, drift, and
  curvature exposed through `Js8RxResult`.
- Added bounded global Costas timing acquisition with fractional-sample
  interpolation and timing-aware soft demodulation for linear sample-clock
  drift, with the estimate exposed through `Js8RxResult`.
- Added bounded quadratic sample-clock drift estimation and timing-aware
  demodulation, with deterministic synthesis and adapter coverage.
- Added coarse sync-quality reporting to detailed scan results and quality-aware
  duplicate replacement while preserving chronological scan ordering.
- Added typed JS8 message semantics for heartbeat, compound,
  compound-directed, directed, and data frames, plus semantic TX audio
  encoding for consumer/UI integration.
- Added JS8Call-aligned max-group soft metrics, per-stream LLR normalization,
  and LDPC erasure retry passes for weak-frame recovery.
- Added a JS8Call-inspired rejected-tone baseline ratio to coarse Costas
  acquisition, with deterministic common-floor regression coverage.
- Coalesced the coarse Costas score per seven-symbol block before applying the
  rejected-tone baseline, matching the oracle's block-level `t/t0` structure.
- Added corpus-test controls for frequency-search width and spacing, allowing
  fast smoke scans without changing full-fidelity defaults.
- Added a corpus-test control for minimum Costas sync quality to diagnose weak
  recordings without changing the default scanner gate.
- Added corpus-test start and duration controls for skipping known-empty audio
  ranges without changing waveform timing.
- Added guard-tone correlation around the 8-FSK band to broaden soft-metric
  noise-floor estimation without changing the public demodulation API.
- Added a bounded adaptive per-symbol soft-metric baseline retry after primary
  JS8 decode failure; the existing primary decode path remains unchanged.
- Added bounded top-three Costas frequency hypotheses per scan window so exact
  FEC/CRC validation can reject an interfering strongest peak.
- Added bounded two-signal residual cancellation: after a valid decode, the
  scanner fits carrier amplitude/phase and subtracts the reconstructed frame
  before retrying the same candidate window; the residual is retained for
  subsequent rolling candidate windows.
- Replaced waterfall-wide brute-force Costas correlation with shared FFT
  acquisition over known Costas symbols. Wide scans now rank spectral carrier
  peaks before invoking exact timing, drift, FEC, and CRC decoding, and exclude
  already-decoded carriers during residual extraction to avoid starving later
  signals.
- Optimized JS8 acquisition hot paths with staged Costas timing refinement,
  reusable waterfall FFT scratch storage, precomputed FFT windows, reduced
  per-hypothesis audio validation, and constant-fraction sample interpolation.
- Added bounded waterfall-wide scanning: consumers can provide an absolute
  frequency band, retain more ranked Costas hypotheses, and extract more than
  two simultaneous signals from one candidate window without changing the
  generic modem contracts.
- Added `Js8ScanConfig::waterfall()` and `Js8RxSession::waterfall()` as the
  primary bounded waterfall-wide configuration and streaming entry point.
- Persisted bounded signal cancellation in `Js8RxSession` across polls, so
  later chunk-fed candidate windows see the cleaned rolling audio buffer.
- Added protocol-local decoding for legacy JS8 Huffman data payloads, with
  explicit validation for the data header, padding sentinel, and code stream.
- Added bounded `Js8MessageReassembler` support for explicit JS8Call
  first/last/data transmission flags without taking ownership of consumer
  scheduling or persistence.

- Replaced per-sample trigonometric calls in tone correlation with phase
  oscillator recurrence, reducing the local JS8 debug test runtime from about
  55 seconds to about 19 seconds without changing scan resolution.
- Rechecked the difficult `A_2_1.wav` oracle fixture at the established
  full-fidelity settings: it remains undecoded, with the optimized release
  scan completing in 176.60 seconds.

### Validation

- Added adversarial tests for timing offsets, frequency offsets, deterministic
  noise, silence/no-decode behavior, duplicate suppression, invalid scan
  policies, linear drift, and curved drift.
- The known JS8Call fixture `A_2_6.wav` decodes as
  `SIUT18l+CDqE` (frame type 3).
- The denser release scan still decodes `A_2_6.wav` as
  `SIUT18l+CDqE` (frame type 3); scan ranking improves duplicate selection
  and observability, not corpus recall by itself.
- Frame-wide lower-envelope baseline subtraction recovered `A_1_4.wav` as
  `Vk4xfHSNwzaX` (frame type 3). A partial release corpus run then decoded six
  of the first seven processed recordings; the complete 9-recording total is
  still pending and is not a claim of full JS8Call noisy-channel
  interoperability.
- Guard-tone baseline estimation passed all deterministic tests but did not
  recover `A_2_1.wav`; the remaining gap requires overlapping spectrum and
  synchronization work rather than a wider scalar tone-floor estimate.

### Remaining work

- Stronger noisy-channel/FEC convergence and candidate ranking.
- Oracle-equivalent multi-signal subtraction and complete rolling-window
  receiver semantics remain future work. The waterfall profile currently
  extracts up to four signals per candidate window using bounded residual
  cancellation.
- Dense JSC compressed payload decoding and long-message reassembly remain
  future work; the new reassembler only joins already-decoded fragments and
  does not implement dense JSC decompression.
- Multi-signal subtraction and complete rolling-window receiver semantics.

[Unreleased]: https://github.com/nicksbar/qsonaut-modems/compare/main...HEAD
