---
id: M4-04
title: photo-library plugin: Android (MediaStore)
milestone: M4 · Mobile apps
labels: [area:mobile, area:android, type:feature]
estimate_days: 4
depends_on: [M4-01]
features: [F2.1]
---

# M4-04 · photo-library plugin: Android (MediaStore)

## Why

The Android half of the camera-roll plugin.

## Scope

- Kotlin plugin: READ_MEDIA_IMAGES/VIDEO (13+) and READ_EXTERNAL_STORAGE (10–12), plus the partial-access path (14+).
- Enumerate MediaStore with generation tracking for incremental changes; `ACCESS_MEDIA_LOCATION` so GPS isn't redacted.
- Same Rust-side API as the iOS plugin.

## Acceptance criteria

- [ ] Incremental scan after 10 new photos takes < 1 s.
- [ ] GPS EXIF is preserved when the permission is granted.

---
_Depends on: M4-01 · Estimate: 4d · Features: F2.1_
