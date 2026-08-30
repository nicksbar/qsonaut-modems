# qsonaut-modems

First-party, UI-independent contracts for shared amateur-radio modem consumers.

This repository is intentionally small. It defines validated audio blocks,
normalized decode events, decode telemetry, and slot timing primitives. It does
not contain third-party protocol implementations and does not own audio
devices, Android lifecycle, GUI state, radio control, TX scheduling, QSO
automation, or persistence.

## Consumers

QSONaut and QSONoid will consume this crate through adapters from
`qsonaut-third-party`. Consumer migration is deliberately deferred until the
adapter repository has been validated independently.

## Development

```sh
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

During local integration, sibling repositories can depend on
`../qsonaut-modems/crates/qsonaut-modems` directly. That path keeps contract
changes immediately visible to the component adapters. Convert it to a pinned
Git revision or published version only when a release workflow is justified.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the ownership boundary
and [docs/CONSUMER-INTEGRATION.md](docs/CONSUMER-INTEGRATION.md) for the
planned QSONaut/QSONoid migration.

The capture-versus-decoder-rate contract is documented in
[docs/AUDIO-BOUNDARY.md](docs/AUDIO-BOUNDARY.md).
