---
id: M6-03
title: Failure scenario suite
milestone: M6 · Hardening & beta
labels: [area:node, type:test]
estimate_days: 3
depends_on: [M6-01]
features: []
---

# M6-03 · Failure scenario suite

## Why

Lock down the cases that could lose data.

## Scope

- Testkit scenarios: offline edits on all devices, delete racing a favorite, keeper joining late, revocation mid-transfer, free-up-space with holders offline, disk-full on the keeper, clock skew of ±1 day.
- Each scenario asserts no original is lost and the copy counts are truthful.

## Acceptance criteria

- [ ] All scenarios pass in CI; each has a short description in `docs/notes/failure-scenarios.md`.

---
_Depends on: M6-01 · Estimate: 3d · Features: —_
