---
id: M4-05
title: Camera-roll backup pipeline and sync conditions
milestone: M4 · Mobile apps
labels: [area:mobile, area:node, type:feature]
estimate_days: 3
depends_on: [M4-03, M4-04]
features: [F2.1, F8.1]
---

# M4-05 · Camera-roll backup pipeline and sync conditions

## Why

Automatic backup is the core mobile use case.

## Scope

- Incremental import from the plugin streams: analyze, add blobs (reference in place when the OS file is stable, otherwise copy), upsert records.
- Sync conditions: Wi-Fi only, charging only, low-battery pause (feed into fetcher pause/resume).
- Back up camera roll toggle; first-run 'found N photos' count (mockup 1d).

## Acceptance criteria

- [ ] A new photo taken on the phone appears on the desktop within 30 s while both apps are in the foreground on the same Wi-Fi.
- [ ] On cellular with Wi-Fi only set, nothing transfers and the UI says why.

---
_Depends on: M4-03, M4-04 · Estimate: 3d · Features: F2.1, F8.1_
