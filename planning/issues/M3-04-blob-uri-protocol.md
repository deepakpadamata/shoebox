---
id: M3-04
title: Blob URI protocol
milestone: M3 · Desktop app
labels: [area:app, type:feature]
estimate_days: 1.5
depends_on: [M3-01]
features: [F2.9]
---

# M3-04 · Blob URI protocol

## Why

Fast image and video loading without passing base64 over IPC.

## Scope

- `shoebox://blob/<hash>?kind=thumb|preview|original` served from the blob store.
- HTTP range support (video seeking), caching headers keyed on the hash (immutable).
- If a blob is missing locally: return 404 quickly and trigger `fetch_now` for previews.

## Acceptance criteria

- [ ] A grid of 1,000 thumbnails loads in < 1 s from a warm store.
- [ ] MP4 seeks without downloading the whole file once it's local.

---
_Depends on: M3-01 · Estimate: 1.5d · Features: F2.9_
