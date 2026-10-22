---
id: M1-03
title: Blob store integration
milestone: M1 · Sync core
labels: [area:node, type:feature]
estimate_days: 2
depends_on: [M1-01]
features: [F2.4]
---

# M1-03 · Blob store integration

## Why

Originals, previews and thumbnails are content-addressed blobs. We need add, get, stream, protect and remove, plus import-by-reference for files we don't own.

## Scope

- Wrap iroh-blobs file store: `add_bytes`, `add_path(copy|reference)`, `get`, `open_range`, `has`, `remove`.
- Tag/pin model so GC never removes referenced or pinned blobs.
- Expose transfer progress events.

## Acceptance criteria

- [ ] A 2 GB file imports by reference without being copied.
- [ ] A blob fetched from a peer is verified (a corrupted transfer is rejected).
- [ ] GC removes only untagged blobs (test).

---
_Depends on: M1-01 · Estimate: 2d · Features: F2.4_
