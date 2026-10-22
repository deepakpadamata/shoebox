---
id: M4-03
title: photo-library plugin: iOS (PhotoKit)
milestone: M4 · Mobile apps
labels: [area:mobile, area:ios, type:feature]
estimate_days: 5
depends_on: [M4-01]
features: [F2.1]
---

# M4-03 · photo-library plugin: iOS (PhotoKit)

## Why

No off-the-shelf Tauri plugin is assumed for camera-roll access, so we write one (05 §6).

## Scope

- Swift plugin: authorization (full and limited), enumerate assets with stable local ids, change observer (persistent change token).
- Read original bytes (incl. iCloud-optimized assets: download from iCloud with progress, or skip with a reason), HEIC/HEVC as-is.
- Live Photos imported as the still only for MVP (pairs are v1.x F2.10).
- Rust side: async stream of `AssetEvent`, `read_original(id) -> file path`.

## Acceptance criteria

- [ ] Enumerates 20k assets in < 10 s.
- [ ] Limited-access mode imports only the selected photos and explains the limitation in the UI.

---
_Depends on: M4-01 · Estimate: 5d · Features: F2.1_
