---
id: M6-02
title: Scale and endurance testing
milestone: M6 · Hardening & beta
labels: [area:node, type:test]
estimate_days: 3
depends_on: [M6-01]
features: []
---

# M6-02 · Scale and endurance testing

## Why

Real libraries are large: 100k items and 600 GB to a new keeper (05 §9).

## Scope

- 100k-item library across 4 devices; 600 GB initial keeper sync with interruption and resume; ETA accuracy.
- Memory and CPU profiling on mobile and on a Raspberry Pi 4 keeper.
- Fix or ticket every regression found.

## Acceptance criteria

- [ ] Keeper initial sync of 600 GB completes on gigabit LAN with ≤ 1 restart-induced re-download of any chunk.
- [ ] Mobile memory stays under 300 MB during a 100k-library sync.

---
_Depends on: M6-01 · Estimate: 3d · Features: —_
