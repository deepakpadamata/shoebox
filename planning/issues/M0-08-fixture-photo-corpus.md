---
id: M0-08
title: Fixture photo corpus
milestone: M0 · Foundations
labels: [area:media, type:test]
estimate_days: 1
depends_on: [M0-02]
features: []
---

# M0-08 · Fixture photo corpus

## Why

Import, sync and bench work need realistic inputs: mixed formats, EXIF, GPS, duplicates and videos.

## Scope

- A script that generates N synthetic JPEGs with varied EXIF (dates, cameras, GPS on/off) and exact duplicates.
- A small checked-in sample set (≤20 MB): HEIC, a RAW+JPEG pair, PNG screenshot, short MP4/MOV, all with a clear license.
- Presets: `small` (200), `medium` (10k), `large` (100k, metadata-only mode for bench).

## Acceptance criteria

- [ ] `cargo xtask fixtures --preset medium` creates the corpus in under 2 minutes.
- [ ] The README lists the sample files' sources and licenses.

---
_Depends on: M0-02 · Estimate: 1d · Features: —_
