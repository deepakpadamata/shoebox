# Contributing

## Build & test

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

The toolchain is pinned to `stable` with rustfmt and clippy (`rust-toolchain.toml`).

## Layout

See [docs/05-prototype-plan.md](docs/05-prototype-plan.md) §1. In short: `crates/` are libraries,
`apps/` are binaries, `tools/` and `xtask/` are developer tooling, `examples/` are throwaway spikes.
`shoebox-core` has no async runtime and no iroh dependency.

## Fixtures

`cargo xtask fixtures --preset small|medium|large` generates a synthetic photo corpus; `cargo xtask samples`
regenerates the checked-in sample files. See [fixtures/README.md](fixtures/README.md).

## Licensing

- Library crates (`crates/*`, `tools/*`, `xtask`, `examples/*`): `Apache-2.0 OR MIT` (`LICENSE-APACHE`, `LICENSE-MIT`).
- Applications (`apps/*`): `AGPL-3.0-only` (`apps/LICENSE-AGPL`).
- A library crate must never depend on an `apps/*` crate (CI enforces this, see M0-03).

## Workflow

- Work is tracked in [planning/issues](planning/issues). Write the failing test first, then the code.
- Assumptions made while implementing are recorded in [docs/notes/assumptions.md](docs/notes/assumptions.md) for review.
