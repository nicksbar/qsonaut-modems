## Summary

<!-- What changed, and why? -->

## Validation

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo metadata --locked --no-deps`
- [ ] `cargo test --workspace --all-targets --locked`
- [ ] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`

## Compatibility

- [ ] Public API changes are documented.
- [ ] Consumer impact is described for QSONaut and QSONoid.
- [ ] No application-specific policy or platform ownership moved into this crate.
