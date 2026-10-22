---
id: M1-04
title: Media import pipeline
milestone: M1 · Sync core
labels: [area:media, type:feature]
estimate_days: 3
depends_on: [M0-04, M0-08]
features: [F2.5, F3.3]
---

# M1-04 · Media import pipeline

## Why

Every device generates its own thumbnails and previews and shares them as blobs, so weak devices never re-encode (F2.5).

## Scope

- Hash the original (BLAKE3, streaming).
- Extract EXIF via `kamadak-exif`: taken_at (with fallback to file mtime), camera, lens, dimensions, orientation, GPS.
- Thumbnail (≈256 px, square crop) and preview (≈2048 px long edge) as JPEG/WebP; orientation applied.
- Formats: JPEG, PNG, HEIC (libheif behind a feature), RAW via embedded preview; video: metadata only (poster in M3-09).
- Pure function API: `analyze(path) -> ImportResult` (no node dependency).

## Acceptance criteria

- [ ] The `medium` fixture corpus imports at ≥ 20 photos/s on an M-series Mac (excluding HEIC).
- [ ] Thumbnails respect EXIF orientation.
- [ ] Unsupported files are reported, not panicked on.

---
_Depends on: M0-04, M0-08 · Estimate: 3d · Features: F2.5, F3.3_
