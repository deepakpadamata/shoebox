---
id: M2-09
title: Keeper release builds
milestone: M2 · Identity, pairing & keeper
labels: [area:keeper, area:infra, type:chore]
estimate_days: 1.5
depends_on: [M2-07]
features: [F4.6]
---

# M2-09 · Keeper release builds

## Why

People need to install the keeper without a Rust toolchain.

## Scope

- Release workflow: static Linux x86_64 + aarch64 (cross/musl), macOS universal binary.
- Checksums + signatures (minisign); a one-line install script that verifies them.

## Acceptance criteria

- [ ] A tagged release produces downloadable, verified binaries for all three targets.
- [ ] The install script works on Debian, Raspberry Pi OS and macOS.

---
_Depends on: M2-07 · Estimate: 1.5d · Features: F4.6_
