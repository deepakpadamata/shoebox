---
id: M0-03
title: CI pipeline with license guard
milestone: M0 · Foundations
labels: [area:infra, type:chore]
estimate_days: 0.5
depends_on: [M0-02]
features: []
---

# M0-03 · CI pipeline with license guard

## Why

Catch regressions early. Make sure AGPL code (apps) can't become a dependency of the permissively licensed library crates.

## Scope

- GitHub Actions: fmt, clippy (`-D warnings`), test on ubuntu-latest + macos-latest.
- `cargo-deny` with a license allow-list; a check that no library crate depends on an `apps/*` crate.
- Cache cargo registry/target.

## Acceptance criteria

- [ ] CI is green on the scaffold.
- [ ] A deliberate AGPL dependency in `shoebox-core` makes CI fail (verified once, then reverted).

---
_Depends on: M0-02 · Estimate: 0.5d · Features: —_
