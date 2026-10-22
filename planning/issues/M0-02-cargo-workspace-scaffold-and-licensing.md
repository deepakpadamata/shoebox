---
id: M0-02
title: Cargo workspace scaffold and licensing
milestone: M0 · Foundations
labels: [area:infra, type:chore]
estimate_days: 1
depends_on: [M0-01]
features: []
---

# M0-02 · Cargo workspace scaffold and licensing

## Why

Set up the repo shape from 05 §1 so every later issue has a home, with the license split from decision #17.

## Scope

- Workspace `Cargo.toml` with pinned iroh crates as workspace deps.
- Empty crates: `shoebox-core`, `shoebox-store-mem`, `shoebox-store-docs`, `shoebox-store-loro` (feature-gated), `shoebox-media`, `shoebox-node`, `shoebox-testkit`; apps `keeper`, `desktop-mobile` (placeholder); `tools/store-bench`.
- `rust-toolchain.toml`, `rustfmt.toml`, clippy config, `.editorconfig`.
- LICENSE files: Apache-2.0 + MIT for library crates, AGPL-3.0 for `apps/*`; `license` field set per crate.
- Short `CONTRIBUTING.md` (build, test, layout).

## Acceptance criteria

- [ ] `cargo build --workspace` and `cargo test --workspace` pass on macOS and Linux.
- [ ] Each crate's `license` metadata matches decision #17.

---
_Depends on: M0-01 · Estimate: 1d · Features: —_
