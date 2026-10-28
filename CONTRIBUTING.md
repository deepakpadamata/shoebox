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

## Disk usage

`target/` grows quickly (every rebuild leaves a ~100–200 MB test binary behind). Run `scripts/prune-target.sh` now and then; the
dev profile already uses reduced debuginfo and no incremental cache.

## Changing a format

From **0.1.0** people have Shoebox data on their devices, and their devices do not all update at once. Everything a
build sends to another build or leaves on disk is listed with its version in
[docs/notes/formats.md](docs/notes/formats.md); `crates/shoebox-node/tests/formats.rs` fails when the source and that
list disagree.

- **Wire protocols** (`shoebox/<name>/<n>`): never change what an existing name means. A changed protocol gets the next
  number, and the release that introduces it serves the old name as well, so release N talks to N-1. The old name goes
  one release later.
- **Signing domains**: a new meaning gets a new domain; never reuse one for a different message.
- **On-disk files**: a change that an older or newer build would misread bumps `CURRENT_FORMAT` and ships a
  `Migration` in `shoebox_node::format` with a test that upgrades real old data. Adding an optional field that old
  builds ignore (`#[serde(default)]`) does not need one.
- Update `formats.md` in the same change, and note the change in `CHANGELOG.md`.

## Fixtures

`cargo xtask fixtures --preset small|medium|large` generates a synthetic photo corpus; `cargo xtask samples`
regenerates the checked-in sample files. See [fixtures/README.md](fixtures/README.md).

## Licensing

- Library crates (`crates/*`, `tools/*`, `xtask`, `examples/*`): `Apache-2.0 OR MIT` (`LICENSE-APACHE`, `LICENSE-MIT`).
- Applications (`apps/*`): `AGPL-3.0-only` (`apps/LICENSE-AGPL`).
- A library crate must never depend on an `apps/*` crate (CI enforces this, see M0-03).

## Workflow

- Write the failing test first, then the code.
- Assumptions made while implementing are recorded in [docs/notes/assumptions.md](docs/notes/assumptions.md) for review.
