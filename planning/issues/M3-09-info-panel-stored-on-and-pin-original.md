---
id: M3-09
title: Info panel, stored-on and pin original
milestone: M3 · Desktop app
labels: [area:ui, type:feature]
estimate_days: 1.5
depends_on: [M3-08]
features: [F3.3, F3.4, F3.5]
---

# M3-09 · Info panel, stored-on and pin original

## Why

Mockup 3b: answers 'where is this stored?' directly.

## Scope

- Inspector with date, file, camera/EXIF, location (if present), size, hash (copy).
- 'Stored on' list per device with kind (original/preview/none).
- 'Keep original on this device' pin toggle (affects fetcher and free-up-space).

## Acceptance criteria

- [ ] Pinning fetches the original and excludes it from free-up-space (test).

---
_Depends on: M3-08 · Estimate: 1.5d · Features: F3.3, F3.4, F3.5_
