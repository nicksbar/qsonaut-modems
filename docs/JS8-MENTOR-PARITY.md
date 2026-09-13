# JS8 mentor parity and validation status

This is a dated capability review, not an on-air interoperability claim. It
compares `qsonaut-js8` with JS8Call Improved revision
`e8a6121d859ba3b678b3485e7a14ed07df1bbee4`, inspected on 2026-09-13.

## Operator modes

| Mentor name | Internal mode | Period | Start delay | Bandwidth | RX threshold | Heartbeat network |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| Slow | JS8-E | 30 s | 500 ms | 25 Hz | -28 dB | Yes |
| Normal | JS8-A | 15 s | 500 ms | 50 Hz | -24 dB | Yes |
| Fast | JS8-B | 10 s | 200 ms | 80 Hz | -22 dB | Yes |
| JS8 40 | JS8-C | 6 s | 100 ms | 160 Hz | -20 dB | No |
| JS8 60 | JS8-I | 4 s | 100 ms | 250 Hz | -18 dB | No |

The mentor enables all five modes and presents simultaneous multi-speed receive
as normal operating behavior. `Js8MultiRxSession` now gives consumers the same
basic choice: monitor all enabled speeds or an explicit subset without making
frequency selection or station selection silently change speed.

## Current operating context

The mentor's default dial-frequency list is 1.8435, 3.578, 5.363, 7.078,
10.130, 14.078, 18.104, 21.078, 24.922, 28.078, 50.318, and 144.178 MHz.
These are application defaults, not modem constants, and remain consumer-owned.
Operators must follow their license, regional band plan, and local conditions.

A PSK Reporter receiver-only API snapshot taken on 2026-09-13 returned 1,718
JS8 reception reports over approximately 46 minutes. About 73% were on 40 m
and 24% on 20 m; 30 m had 26 reports and the remaining bands were sparse in
that short sample. PSK Reporter labels these as JS8 but does not report the
JS8 speed, so it cannot answer whether a station used Normal, Fast, JS8 40,
Slow, or JS8 60. This snapshot is useful for choosing a receive-only test band,
not as a permanent band-activity claim.

## Capability matrix

| Area | Current state | Remaining proof or work |
| --- | --- | --- |
| Five physical modes | Implemented with current names and timing metadata; current mentor data vectors match every mode | Expand fresh vectors across more frame and message types |
| Waterfall acquisition | FFT-ranked 200-3000 Hz profile, up to 8 hypotheses | Compare weak-signal candidate ordering with mentor |
| Simultaneous carriers | Three-channel test and unequal-power overlapping Fast/JS8 60 test pass; profile allows four | Add combined drift, timing offset, and noisy-channel fixtures |
| Multi-speed RX | Batch and chunk-fed APIs; all five modes by default | Measure sustained CPU and cancellation latency in consumers |
| Short frame/message types | Compact heartbeat, compound, directed, data, and legacy Huffman support; all 32 command policies are exposed | Expand current mentor protocol vectors across message types |
| Long/free text | Bounded fragment reassembler when caller supplies flags | Dense JSC decompression and mentor-equivalent flag pipeline |
| Group behavior | Current reserved groups, including `@ALLCALL`, `@HB`, and `@POTA`, round-trip through directed frames | Verify fresh mentor vectors; reply and rate-limit policy remains consumer-owned |
| Noisy-channel corpus | Several historical fixtures decode | `A_2_1.wav` still fails; complete and pin the full corpus total |
| TX scheduling | Waveform generation plus exposed period/start-delay metadata | Consumer-owned slot scheduling and mentor acceptance tests |
| RF validation | None claimed | Receive-only known-transmission test, then separately authorized TX test |

## Validation order

1. Expand the fresh current-mentor data vector into representative heartbeat,
   compound, directed, and compressed message vectors.
2. Complete the external media-corpus result and retain failures as regression
   fixtures where redistribution permits.
3. Run a receive-only overnight capture on 40 m near 7.078 MHz and retain raw
   48 kHz audio, normalized 12 kHz audio, UTC timestamps, and decoder telemetry.
4. Compare the same recording with JS8Call Improved and `qsonaut-js8`, including
   candidates rejected before CRC, timing offsets, SNR, frequency, and speed.
5. Treat transmission validation as a separate, explicitly authorized safety
   step after receive parity is understood.
