# qsonaut-modems repository instructions

## Purpose

This repository contains first-party, permissively licensed contracts shared by
QSONaut, QSONoid, and future station consumers. It is not a GUI, audio-device
driver, radio-control library, protocol implementation, or QSO application.

## Source organization

- `crates/qsonaut-modems/src/lib.rs` is the small public facade and re-export
  list. Keep it free of implementation details.
- `audio.rs` owns validated mono PCM blocks, pure aligned-window extraction,
  and audio-boundary errors.
- `events.rs` owns normalized decode events, batches, and telemetry.
- `timing.rs` owns slot descriptions and reusable slot gating.
- `docs/ARCHITECTURE.md` defines ownership boundaries.
- `docs/CONSUMER-INTEGRATION.md` is the migration contract for consumers.
- `crates/js8/src/lib.rs` is only the JS8 crate facade. JS8 implementation
  behavior belongs in focused sibling modules such as `alphabet.rs`,
  `frame.rs`, `fec.rs`, `costas.rs`, `mode.rs`, `synth.rs`, `sync.rs`, and
  `decode.rs`.

Keep new behavior in the narrowest logical module. Do not grow `lib.rs` into a
single implementation file. Add module-level tests beside the behavior they
cover and use public integration tests only for cross-module contracts.

For `qsonaut-js8`, do not add protocol logic, large constant tables, or tests
to `lib.rs` or a monolithic `mod.rs`. Keep public exports in the facade and
place each physical/protocol responsibility in its own file. Split a module
before it becomes difficult to review rather than waiting for a later cleanup.

## Design rules

- Keep this crate independent of `eframe`, `cpal`, Android/JNI, serial/radio
  code, QSONaut GUI state, QSO logging, network clients, and TX scheduling.
- Audio must carry an explicit sample rate; never silently assume 48 kHz or
  12 kHz in a generic contract.
- Preserve the distinction between audio frequency offsets and RF frequency.
- Contracts must be usable from desktop and Android Rust code.
- Prefer additive, backwards-compatible API changes. Document intentional
  breaking changes before making them.
- Keep timing primitives policy-neutral: consumers own clocks, buffering,
  cancellation, TX-slot suppression, and worker lifetimes.
- Do not add protocol-specific result types here. First-party protocol
  implementations belong in sibling crates such as `qsonaut-js8`; external
  protocol adapters belong in `qsonaut-third-party`.

## Consumer safety boundary

This crate must never arm or transmit. TX safety, global disarm, PTT handling,
late-worker suppression, and operator workflow remain in each consumer until a
separate reviewed abstraction proves otherwise.

## Required checks

Run from this repository root, not `/home/nick/RigForge`:

```sh
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

For public API changes, also inspect generated documentation:

```sh
cargo doc --workspace --no-deps
```

Do not claim QSONaut or QSONoid compatibility from this repository alone. A
consumer migration requires fixture parity, result comparison, and the
consumer's own desktop or Android validation.

## Change procedure

1. Read the relevant architecture and consumer-integration docs.
2. Make the smallest module-local change.
3. Add deterministic tests for validation, empty input, and boundary behavior
   where applicable.
4. Run all required checks.
5. Update docs for public contracts or changed ownership.
6. Report consumer repositories as untouched unless an explicit integration
   task authorizes changes there.
