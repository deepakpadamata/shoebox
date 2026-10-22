---
id: M1-14
title: Loro backend spike
milestone: M1 · Sync core
labels: [area:store, type:spike]
estimate_days: 3
depends_on: [M0-07, M1-08]
features: []
---

# M1-14 · Loro backend spike

## Why

The alternative to iroh-docs for decision #7: Loro documents synced over gossip + blobs.

## Scope

- Feature-gated `shoebox-store-loro` implementing `LibraryStore` (albums can wait).
- Snapshot + incremental updates stored as blobs; update announcements via gossip.
- Enough to run the conformance suite's photo/holdings parts and the benchmark.

## Acceptance criteria

- [ ] Photo and holdings conformance tests pass.
- [ ] A write-up of the gaps and the effort needed to finish.

---
_Depends on: M0-07, M1-08 · Estimate: 3d · Features: —_
