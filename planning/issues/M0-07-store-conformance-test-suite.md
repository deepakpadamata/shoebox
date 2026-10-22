---
id: M0-07
title: Store conformance test suite
milestone: M0 · Foundations
labels: [area:store, type:test]
estimate_days: 1.5
depends_on: [M0-06]
features: []
---

# M0-07 · Store conformance test suite

## Why

A single behavioural spec that every backend (mem, docs, loro) has to pass.

## Scope

- A generic test suite `conformance::run::<B: StoreBackend>()`, exposed behind a `testing` feature.
- Scenarios: idempotent upsert, paging by time, favorite LWW, delete/restore/purge, album add-wins, holdings single-writer, at_risk counts, change-stream events.
- Two-replica scenarios: concurrent edits, then sync, then converge.

## Acceptance criteria

- [ ] The mem backend passes the whole suite.
- [ ] The suite can be invoked from another crate with three lines.

---
_Depends on: M0-06 · Estimate: 1.5d · Features: —_
