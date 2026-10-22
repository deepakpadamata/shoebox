---
id: M1-11
title: Holdings and copy counts
milestone: M1 · Sync core
labels: [area:node, area:core, type:feature]
estimate_days: 2
depends_on: [M1-08]
features: [F2.6, F3.4, F7.1, F7.2]
---

# M1-11 · Holdings and copy counts

## Why

Copy count is the product's core promise: "if I lose this phone, what do I lose?"

## Scope

- Each node publishes `HoldingsDelta` when blobs are added or removed (batched).
- `holders(blob)`, `copy_count(photo)`, `at_risk(min_copies)` using the library redundancy target (default 2).
- Library-level setting for the redundancy target.

## Acceptance criteria

- [ ] Copy counts are correct across 3 nodes after adds, removes and an offline node (testkit).
- [ ] `at_risk` for a 100k library returns in < 200 ms.

---
_Depends on: M1-08 · Estimate: 2d · Features: F2.6, F3.4, F7.1, F7.2_
