---
id: M5-07
title: Save shared photo to my library
milestone: M5 · Shared albums
labels: [area:node, type:feature]
estimate_days: 1
depends_on: [M5-01]
features: [F6.5]
---

# M5-07 · Save shared photo to my library

## Why

Keep something from a shared album even after leaving it.

## Scope

- `save_to_library(items)`: upsert `PhotoRecord`s referencing the same hashes; blobs fetched according to my role.

## Acceptance criteria

- [ ] Saving an already-fetched item is instant and creates no duplicate blob.

---
_Depends on: M5-01 · Estimate: 1d · Features: F6.5_
