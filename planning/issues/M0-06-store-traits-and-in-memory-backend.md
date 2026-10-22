---
id: M0-06
title: Store traits and in-memory backend
milestone: M0 · Foundations
labels: [area:core, area:store, type:feature]
estimate_days: 1.5
depends_on: [M0-05]
features: []
---

# M0-06 · Store traits and in-memory backend

## Why

The `LibraryStore` / `AlbumStore` / `StoreBackend` boundary is what keeps the iroh-docs vs Loro choice (#7) cheap.

## Scope

- Traits as sketched in 05 §2 (async-trait, `BoxStream<Change>`), including `Capability` and the signed-device-list methods (signature verification stubbed until M2-01).
- `shoebox-store-mem`: a full implementation using `core::merge`, with a simulated replica-to-replica `sync_with` for tests.
- `Page`, `TimeRange` and cursor types.

## Acceptance criteria

- [ ] The in-memory backend implements every trait method.
- [ ] Rustdoc for each trait method states its merge/idempotency contract.

---
_Depends on: M0-05 · Estimate: 1.5d · Features: —_
