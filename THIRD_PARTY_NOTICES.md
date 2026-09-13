# Third-party notices and provenance

## JS8Call and WSJT-X

The `qsonaut-js8` crate exists because of the work of the JS8Call and WSJT-X
communities. We are grateful to the people who designed, implemented,
documented, tested, and maintained those projects. This crate does not claim
credit for the original JS8 modem, its protocol design, its message semantics,
or the engineering work that made the mode usable.

The current compatibility reference is JS8Call Improved:

- Project: [JS8Call Improved](https://github.com/JS8Call-improved/JS8Call-improved)
- Revision consulted: `e8a6121d859ba3b678b3485e7a14ed07df1bbee4`
- License: GNU General Public License version 3, as identified by its
  `COPYING` and source notices

The original implementation foundation and external media corpus use the
historical JS8Call repository:

- Project: [JS8Call](https://github.com/js8call/js8call)
- Revision consulted: `a7ff1be0b389d287fdc56e2ea0d06962aa68127d`
- License: GNU General Public License version 3 or later, as identified by
  JS8Call's `COPYING` and source notices

JS8Call's own project notice describes it as a derivative of WSJT-X and states
that it is not supported or endorsed by the WSJT-X development group. The
original WSJT-X and JS8Call authors and contributors retain the rights and
credit for their respective work. QSONaut is not affiliated with or endorsed
by either project.

### What this repository contains

`qsonaut-js8` is an independently written Rust implementation. It was
constructed by studying the upstream source and documentation, comparing
observable behavior, and writing focused Rust modules and tests for this
repository. In particular, upstream material was used as a compatibility
oracle for:

- protocol and message behavior;
- mode/sample-rate/frame constants;
- Costas synchronization sequences, alphabet, CRC, and LDPC interoperability
data;
- generated tone and audio behavior; and
- optional external JS8Call media-corpus comparisons.

Those facts and interoperability data are not original QSONaut inventions.
The Rust expression, module structure, adapter contracts, scanner, DSP
implementation, tests, and integration documentation in this repository were
written for `qsonaut-modems`; they are not copied JS8Call or WSJT-X source.

### What this repository does not contain

The tracked repository does **not** include:

- JS8Call or WSJT-X C, C++, Fortran, Qt, build-system, or application source;
- a vendored JS8Call/WSJT-X checkout or native-library dependency;
- the external JS8Call WAV corpus or other upstream media fixtures; or
- copied upstream source files renamed as Rust files.

The oracle checkout used during development was kept outside this repository
under `~/.cache/rigforge/oracles/js8call`. Current mentor review uses a second
external checkout under `~/.cache/rigforge/oracles/js8call-improved`. The
optional corpus test accepts a path to the historical external checkout and
skips it when no path is provided.

## Licensing boundary

The generic `qsonaut-modems` contract crate remains MIT-licensed and
protocol-neutral. The `qsonaut-js8` implementation crate is deliberately
licensed under GPL-3.0-or-later and carries its own license notice. That
license choice and the attribution above do not transfer ownership of JS8Call,
WSJT-X, or their original source to QSONaut contributors.

This notice is a provenance summary, not a replacement for the upstream
licenses or legal advice. When redistributing software that combines this
crate with upstream JS8Call/WSJT-X material, review and preserve all applicable
copyright and license notices for the material actually distributed.
