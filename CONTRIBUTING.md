# Contributing

Keep presentation reusable and authority-free. Process lifecycle and adapter
selection belong only in the standalone binary or another explicit composition
root. Source, history, persistence, discovery, and clipboard effects must flow
through their Vellum contracts and adapters.

Required local gate:

```sh
cargo fmt --check
cargo test --locked --offline
cargo test --all-targets --features standalone-macos --locked --offline
cargo clippy --all-targets --features standalone-macos --locked --offline -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --features standalone-macos --locked --offline
```

Add behavioral or model evidence for state transitions and fault boundaries.
Keep failures content-safe, bounds explicit, and uncertainty typed. Do not add
crates.io publication metadata or remote effects without an approved release
change.
