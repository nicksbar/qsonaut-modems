# Changelog

All notable changes to `qsonaut-modems` are documented here.

## [Unreleased]

### Added

- Added explicit JS8Call and WSJT-X attribution, provenance, licensing-boundary,
  and non-bundled-source notices. The Rust implementation takes no credit for
  the original projects and identifies the upstream compatibility material it
  reproduces.
- Added the first-party `qsonaut-js8` crate under `crates/js8`.
- Added JS8 TX/RX support for Normal, Fast, Turbo, Slow, and Ultra modes.
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
- Added corpus-test start and duration controls for skipping known-empty audio
  ranges without changing waveform timing.
- Added guard-tone correlation around the 8-FSK band to broaden soft-metric
  noise-floor estimation without changing the public demodulation API.
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

- Higher-order clock compensation.
- Stronger noisy-channel/FEC convergence and candidate ranking.
- Multi-signal subtraction and complete rolling-window receiver semantics.

[Unreleased]: https://github.com/nicksbar/qsonaut-modems/compare/main...HEAD
