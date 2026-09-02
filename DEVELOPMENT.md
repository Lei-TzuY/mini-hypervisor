# Development

## Required checks

Before a change is considered mergeable, run:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

KVM-dependent behavior must detect an unavailable `/dev/kvm` separately from a VMM regression. Pure validation and parser/device tests must never be skipped merely because KVM is unavailable.

## Scope discipline

Each development round should take one bounded vertical slice. Avoid introducing empty modules or speculative interfaces. The current baseline intentionally stops before guest physical memory.
