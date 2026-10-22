---
id: M1-05
title: iroh-docs LibraryStore backend
milestone: M1 · Sync core
labels: [area:store, type:feature]
estimate_days: 4
depends_on: [M0-07, M1-01]
features: []
---

# M1-05 · iroh-docs LibraryStore backend

## Why

The default metadata backend for the prototype (decision #7).

## Scope

- Key layout from 03 §Data model (`photos/`, `albums/`, `devices/`, `holdings/`), postcard values.
- Read paths apply `core::merge` across concurrent per-author entries.
- `subscribe()` maps iroh-docs live events to `Change`.
- `sync_with(peer)` and capability export/import (write/read) for the library namespace.
- Index for `photos_by_time` (secondary key by taken_at).

## Acceptance criteria

- [ ] The whole conformance suite passes against the docs backend.
- [ ] The testkit 3-node convergence scenario passes on this backend.

## Notes

Track query performance numbers here; the full comparison happens in M1-15.

---
_Depends on: M0-07, M1-01 · Estimate: 4d · Features: —_
