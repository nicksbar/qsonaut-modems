# JS8 protocol foundation

This document is the research and implementation foundation for the first-party
`qsonaut-js8` crate. The generic `qsonaut-modems` contract crate remains
protocol-neutral; JS8 implementation code belongs in that sibling crate.

The first pure frame-encoding milestone now exists in `qsonaut-js8`; audio
synthesis, synchronization, decoding, and consumer integration remain future
milestones.

## Decision summary

JS8 is best understood as two layers:

1. **A compact radio frame and modem**: 8-FSK tones, Costas synchronization,
   CRC, LDPC, timing/frequency acquisition, and weak-signal decoding.
2. **A chat-like message protocol**: CQ, heartbeat, directed calls, short text,
   directed commands, and inbox/store-and-forward behavior.

The second layer resembles an IRC room carried over a broadcast RF channel. It
is not an IRC client/server protocol: there is no central server, delivery is
not guaranteed, and all text must pass through the weak-signal modem.

The implementation is a native Rust JS8 modem in the `qsonaut-js8` crate,
using the official JS8Call implementation as the compatibility oracle.
A Rust implementation is preferable to binding the whole Qt application, but it
must be proven against upstream before it is presented as interoperable.

This work is an independent Rust implementation, not a claim of authorship of
JS8, JS8Call, or WSJT-X. We thank the JS8Call and WSJT-X authors and
contributors for the original modem, protocol, documentation, and engineering
work. Upstream source and behavior informed the compatibility work; the
implementation in this repository was written as focused Rust modules rather
than by copying upstream C, C++, Fortran, Qt, or build-system files. See
[`THIRD_PARTY_NOTICES.md`](../THIRD_PARTY_NOTICES.md) for the detailed
provenance and repository-content boundary.

## Boundary rules

### Generic contracts (`qsonaut-modems`)

This repository may document and eventually expose only generic contracts such
as:

- audio blocks with explicit sample rates;
- normalized decode events and telemetry;
- slot timing and gating;
- generic modem identifiers;
- generic TX/RX capabilities if a future adapter demonstrates that they are
  needed by more than one protocol.

The generic contract crate must not contain:

- JS8 alphabet, frame-type, command, or callsign types;
- JS8 tone tables, Costas arrays, LDPC matrices, CRC implementations, or DSP;
- JS8-specific SNR, synchronization, or decode-result types;
- audio capture, device, radio, PTT, scheduling, QSO, or IRC/network logic.

### First-party implementation (`qsonaut-js8`)

The JS8 crate owns:

- JS8 protocol packing and unpacking;
- waveform synthesis and decoding;
- upstream source and native dependency licensing;
- sample-rate and frame-shape validation;
- optional translation to `qsonaut_modems::DecodeBatch` and
  `qsonaut_modems::DecodeEvent` at an integration boundary;
- deterministic fixtures and upstream compatibility tests.

Consumers continue to own capture, resampling, rolling buffers, slot policy,
worker lifetime, cancellation, TX safety, UI, logging, and message workflow.

## Upstream oracle

The primary oracle is the official JS8Call repository:

- Repository: <https://github.com/js8call/js8call>
- Revision inspected for this foundation: `a7ff1be0b389d287fdc56e2ea0d06962aa68127d`
- License: GPL-3.0, see `COPYING`

The oracle is not only a source of constants. It is the behavioral reference
for encoded tones, generated audio, accepted timing/frequency offsets, decoder
results, message semantics, and mode behavior.

### Local oracle checkout

Keep a detached checkout outside the tracked repositories for source
inspection, oracle-tool development, and fixture generation. The working
checkout used during this research is:

```text
~/.cache/rigforge/oracles/js8call
```

It is pinned to:

```text
a7ff1be0b389d287fdc56e2ea0d06962aa68127d
```

The checkout is a development tool, not a Cargo dependency and not a source
directory to copy into `qsonaut-modems`. Future scripts should accept an
`JS8CALL_ORACLE_DIR` environment variable and verify the expected revision
before generating or updating fixtures. A fixture update must record both the
oracle revision and the generator version.

No JS8Call or WSJT-X source checkout, native source file, or external media
fixture is tracked in this repository. The optional media test reads files
from the external oracle checkout only when `JS8CALL_MEDIA_TESTS` is set.

### Oracle source map

| Upstream path | What to learn | Rust implementation consequence |
| --- | --- | --- |
| `commons.h` | Global sample rate, frame size, symbol count, mode constants, decoder buffer shape | Define internal JS8 mode parameters and explicit 12 kHz input validation. |
| `JS8.hpp` | `JS8::encode`, Costas arrays, decode event fields, decoder public shape | Reproduce the encoder contract and normalize decoder metadata. |
| `JS8.cpp` | Current C++ encoder, decoder, synchronization, downsampling, baseline, LDPC, signal subtraction | Port or independently implement behavior in small Rust modules; do not copy the Qt worker boundary. |
| `JS8Submode.cpp/.hpp` | User-facing mode names, symbol/sample counts, durations, start delays, bandwidth and tone spacing | Create adapter-local mode metadata and slot recommendations. |
| `varicode.cpp/.h` | Message commands, heartbeat/CQ parsing, directed command packing, extended characters | Implement the message layer separately from the physical codec. |
| `js8a_module.f90`, `js8b_module.f90`, `js8c_module.f90`, `js8e_module.f90`, `js8i_module.f90` | Historical/reference Fortran mode algorithms and constants | Use for cross-checking the C++ conversion and resolving behavior that is not obvious in `JS8.cpp`. |
| `lib/js8*_decode.f90` | Historical decoder stages and numerical behavior | Use as a second oracle when porting synchronization or decoder math. |
| `CMakeLists.txt` and `js8call.pro` | Actual source lists, compiler requirements, FFTW linkage, Qt coupling | Identify dependencies to avoid in a Rust implementation and record what was intentionally not ported. |
| `README.md`, `docs/`, and `contrib/` | User-visible mode semantics and protocol/API behavior | Validate terminology and avoid confusing JS8 RF frames with JS8Call TCP/API messages. |
| `COPYING` and source notices | GPL obligations and attribution | Keep the adapter’s upstream provenance and notices complete. |

### Current constants to preserve

At the inspected revision:

- decoder input rate: **12,000 samples/second**;
- receive ring/frame horizon: **60 seconds**;
- frame contains **79 symbols**;
- modes A/B/C/E/I use symbol sample counts of 1920/1200/600/3840/384;
- nominal TX durations are 15/10/6/30/4 seconds for A/B/C/E/I;
- current upstream build enables A/B/C/E and disables I through `commons.h`;
- mode A uses the original Costas arrays; the other modes use modified arrays;
- the modem occupies an audio passband rather than an RF frequency. The decoded
  frequency is an audio offset in Hz.

These values are revision-scoped. They must be extracted into tests and not
silently treated as universal constants if upstream changes them.

## Protocol and waveform model

### Message/frame layer

The current encoder path in `JS8.cpp` establishes this core shape:

1. Convert 12 characters using JS8's 64-character alphabet
   `0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz-+`.
2. Pack 72 message bits.
3. Add a 3-bit frame/message type.
4. Add a 12-bit CRC-12 over the first 75 bits.
5. Produce an 87-bit information word.
6. Apply the JS8 LDPC `(174,87)` code.
7. Split the result into 58 three-bit data symbols.
8. Surround data with three 7-symbol Costas synchronization arrays, for 79
   total channel symbols.

The exact bit order, CRC augmentation, parity matrix, Costas arrays, alphabet,
and frame-type placement are interoperability-critical. A plausible-looking
implementation is not sufficient.

The message text layer then interprets the decoded 12-character payload and
frame type as CQ, heartbeat, directed text, or a directed command. `varicode.cpp`
is the oracle for command names and packing rules. The TCP API message names
such as `RX.TEXT` and `TX.TEXT` are JS8Call application API concepts, not the
RF frame format.

### Physical layer

The waveform path converts channel symbols into 8-FSK audio at 12 kHz. The
mode-specific symbol duration changes tone spacing and total frame duration.
The decoder must perform substantially more than symbol slicing:

- spectrum construction and candidate search;
- Costas correlation and synchronization scoring;
- time-offset search;
- fine frequency-offset estimation;
- narrow-band downsampling/filtering;
- tone metrics and soft-decision likelihoods;
- LDPC belief-propagation decoding;
- CRC validation;
- duplicate suppression and optional signal subtraction;
- SNR and quality estimation.

The decoder's numerical behavior is part of compatibility. FFT normalization,
window functions, floating-point order, candidate thresholds, and offset
conventions can change decode results even when the high-level algorithm looks
correct.

## Oracle-to-Rust translation plan

The implementation should be split into testable, protocol-local layers. These
names are proposed for `qsonaut-js8`; they are JS8-crate APIs, not generic
contracts for the `qsonaut-modems` crate.

```text
js8/
  mode.rs       mode A/B/C/E/I parameters and revision-scoped capabilities
  alphabet.rs   64-character mapping and 12-character payload validation
  frame.rs      frame type, CRC-12, bit packing, tone-symbol packing
  fec.rs        JS8 (174,87) parity and bounded min-sum decoder
  costas.rs     original/modified Costas arrays and correlation helpers
  synth.rs      symbols -> 12 kHz real audio
  sync.rs       aligned tone correlation, then candidate search and refinement
  metrics.rs     normalized soft tone likelihoods for aligned RX
  decode.rs     frame reconstruction, CRC, and future duplicate/subtraction pipeline

  messages.rs   CQ/heartbeat/directed-command semantics
  adapter.rs    AudioBlock -> DecodeBatch and normalized DecodeEvent
```

The first implementation should keep all internal types private to the JS8
crate. Only `adapter.rs` should translate into generic first-party types. That prevents
JS8-specific details from leaking into the generic contract crate.

| Oracle behavior | Rust location | Proof required |
| --- | --- | --- |
| Alphabet and character rejection | `alphabet.rs` | Exhaustive alphabet round trip and invalid-character tests. |
| 75-bit payload + type + CRC | `frame.rs` | Golden packed-bit vectors from the oracle; CRC failure tests. |
| LDPC parity generation | `fec.rs` | Every oracle-generated codeword satisfies the parity checks. |
| Tone sequence and Costas placement | `frame.rs`, `costas.rs` | Exact 79-symbol golden vectors for every enabled mode. |
| Audio phase/tone synthesis | `synth.rs` | Sample-level or tolerance-based waveform comparison at fixed frequency. |
| Candidate timing/frequency | `sync.rs` | Offset sweeps with known generated signals and noise. |
| Decode event metadata | `decode.rs`, `adapter.rs` | Compare message, SNR, `xdt`, and audio frequency to oracle output. |
| CQ/directed command behavior | `messages.rs` | Corpus tests from upstream `varicode` behavior and edge cases. |

## Oracle harness and fixtures

Before implementing the full decoder, build a repeatable oracle harness outside
the generic contract crate. It may initially be a development tool beside
`qsonaut-js8` or a separate ignored workspace tool, but its inputs and
outputs must be checked into a reviewed fixture location once stable.

The harness should produce versioned records containing:

- upstream commit;
- mode and submode;
- 12-character payload;
- frame type;
- base audio frequency;
- sample rate;
- generated tone symbols;
- generated PCM or IQ samples, if the oracle exposes them;
- decoder input fixture and any injected SNR/time/frequency offsets;
- decoded message;
- SNR;
- time offset;
- audio-frequency offset;
- quality and decoder status where available.

Use at least these fixture classes:

1. **Pure frame vectors**: known payloads, frame types, packed bits, parity, and
   79 tones. These should be architecture-independent and exact.
2. **TX round trips**: oracle-generated audio decoded by the oracle and later by
   Rust.
3. **Rust-to-oracle round trips**: Rust-generated audio accepted by the oracle.
4. **Offset sweeps**: time and audio-frequency shifts around nominal alignment.
5. **Noise/SNR sweeps**: deterministic seeded noise, not random test noise.
6. **Collision tests**: multiple signals and subtraction behavior.
7. **No-decode tests**: silence, malformed frames, wrong CRC, and unsupported
   sample rates.
8. **Message semantics**: CQ, heartbeat, callsign-directed messages, commands,
   extended characters, and maximum-length payloads.

Never use only a successful clean loopback as evidence. A modem that decodes its
own waveform but not the oracle waveform is not interoperable.

## Sampling and adapter boundary

The current shared audio contracts can carry JS8 input as an explicit
`AudioBlock` at 12 kHz. The consumer may capture at another rate and use the
shared normalizer or another reviewed resampler before invoking the adapter.
The adapter must:

- reject unsupported rates rather than silently resampling;
- reject non-finite samples;
- define whether it accepts a complete slot, a rolling decode window, or a
  streaming chunk;
- preserve audio frequency offset separately from RF frequency;
- return an empty `DecodeBatch` for valid silence;
- avoid owning capture, clocks, slot scheduling, cancellation, or PTT safety.

JS8's differing durations mean the adapter must expose mode metadata to its
consumer through adapter-local APIs. `SlotSpec` can represent the selected
slot, but should not be expanded with JS8-specific fields in this repository.

For RX normalization, the natural generic mapping is:

- decoded text -> `DecodeEvent.message`;
- oracle SNR -> `DecodeEvent.snr_db`;
- oracle `xdt` -> `DecodeEvent.delta_time_seconds`;
- oracle audio frequency -> `DecodeEvent.audio_frequency_hz`;
- adapter-local mode/frame/message details remain private until a demonstrated
  cross-protocol contract is justified.

## Build and licensing position

The official JS8Call source is GPL-3.0. A direct port, copied source, or linked
native implementation must retain the applicable license and attribution. The
`qsonaut-js8` crate must record:

- exact upstream JS8Call revision;
- whether code was copied, ported, or linked;
- all native dependencies and their licenses;
- generated tables or matrices and their provenance;
- corresponding source obligations for distributed binaries.

A Rust reimplementation does not remove the need for compatibility and
provenance review. It also does not automatically make ported algorithmic code
permissively licensed. The `qsonaut-js8` crate is GPL-3.0-or-later and carries
this provenance review; the generic contract crate remains protocol-neutral.

## Milestones and stop conditions

### Milestone 0: protocol notebook and oracle vectors

Complete the source map, fixture schema, and exact pure-frame vectors before
adding public Rust APIs. Stop if the oracle's bit ordering or mode constants
cannot be reproduced exactly.

### Milestone 1: pure Rust frame codec — complete

Implement alphabet, payload/type packing, CRC, LDPC encode/decode, Costas arrays,
and 79-tone vectors. Do not claim audio modem support yet.

Exit criteria:

- exact oracle frame vectors for every enabled mode;
- malformed input and deterministic CRC/frame tests;
- deterministic tests on the supported toolchains.

The first implementation lives in the GPL-licensed `qsonaut-js8` crate. Its
facade is intentionally small; alphabet packing, CRC, FEC, Costas data, frame
assembly, mode metadata, and synthesis are separate modules.

### Milestone 2: transmitter and clean loopback — in progress

The initial transmitter now generates continuous-phase 12 kHz mono PCM for all
known mode sample counts. Compare Rust-generated audio with the oracle and
decode both directions before claiming waveform interoperability.

Exit criteria:

- oracle accepts Rust audio;
- Rust accepts oracle audio;
- frequency and timing conventions are documented;
- no consumer or QSONaut changes are needed.

### Milestone 3: synchronization and single-signal RX — started

Add candidate search, timing/frequency refinement, soft metrics, and decode
metadata. The first aligned 8-FSK correlator now recovers channel tones from a
complete 12 kHz window when the caller supplies the mode and audio-frequency
offset. Costas-based timing search now scans one symbol period and recovers a
frame boundary with up to one symbol of leading uncertainty. Bounded Costas
frequency search now estimates the absolute audio frequency on a caller-
provided grid. The soft metric layer now emits normalized per-symbol tone
likelihoods and converts them to systematic-codeword bit LLRs. A bounded
min-sum `(174,87)` decoder validates clean codewords and corrects the verified
single-hard-bit case. Frame reconstruction now unpacks the 12-character
payload, extracts the frame type, and validates CRC. Fine drift correction,
robust noisy-channel convergence, and multi-signal subtraction are not
implemented yet. Keep multi-signal subtraction disabled until single-signal
parity is proven.

### Milestone 4: integration and additional modes

Extend `qsonaut-js8`, then implement B/C/E one at a time. Treat I as
unsupported until upstream enables it and fixtures exist. Add normalized events
and explicit errors without changing generic contracts.

### Milestone 5: multi-signal and message behavior

Add subtraction, duplicate policy, directed-message parsing, heartbeat/CQ
semantics, and application-facing helpers only in the adapter boundary.

## What not to do

- Do not add `Js8DecodeEvent`, `Js8Frame`, `Js8Mode`, or JS8 command types to
  the generic `qsonaut-modems` crate merely to start experimentation; keep
  them in `qsonaut-js8`.
- Do not add JS8 to the existing WSJT dispatcher until the implementation has a
  compatible adapter API and verified fixtures.
- Do not treat `js8call_lib` as a modem dependency; it is a JS8Call TCP/JSON
  client.
- Do not use `rusty_v8`; it is a JavaScript engine binding, not JS8 DSP.
- Do not copy the entire Qt application or its worker lifecycle into a Rust
  adapter.
- Do not claim interoperability from protocol diagrams or a self-loopback
  alone.
- Do not let consumer slot scheduling or radio safety enter the modem library.

## References

- Official source: <https://github.com/js8call/js8call>
- Oracle revision: `a7ff1be0b389d287fdc56e2ea0d06962aa68127d`
- JS8Call license: <https://github.com/js8call/js8call/blob/main/COPYING>
- JS8Call API wrapper (not a modem): <https://crates.io/crates/js8call_lib>
- First-party architecture: [ARCHITECTURE.md](ARCHITECTURE.md)
- Audio boundary: [AUDIO-BOUNDARY.md](AUDIO-BOUNDARY.md)
- Consumer integration: [CONSUMER-INTEGRATION.md](CONSUMER-INTEGRATION.md)
 - QSONaut JS8 integration: [QSONAUT-JS8-INTEGRATION.md](QSONAUT-JS8-INTEGRATION.md)
