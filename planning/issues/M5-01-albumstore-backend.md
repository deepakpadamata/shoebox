---
id: M5-01
title: AlbumStore backend
milestone: M5 · Shared albums
labels: [area:store, type:feature]
estimate_days: 3
depends_on: [M1-15, M2-03]
features: [F6.1]
---

# M5-01 · AlbumStore backend

## Why

Shared albums are their own replicated spaces with membership and roles (03 §Sharing).

## Scope

- Implement `AlbumStore` + `StoreBackend::create_album/join_album` on the chosen backend: `meta`, `items/`, `members/`, `events/`.
- Extend the conformance suite with album scenarios (concurrent adds, member changes, events ordering).

## Acceptance criteria

- [ ] The album conformance suite passes.
- [ ] An album with 50k items syncs to a new member in < 2 min on LAN (metadata only).

---
_Depends on: M1-15, M2-03 · Estimate: 3d · Features: F6.1_
