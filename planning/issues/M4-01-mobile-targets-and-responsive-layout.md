---
id: M4-01
title: Mobile targets and responsive layout
milestone: M4 · Mobile apps
labels: [area:mobile, area:ui, type:feature]
estimate_days: 3
depends_on: [M3-17]
features: []
---

# M4-01 · Mobile targets and responsive layout

## Why

Same Tauri app on iOS 16+ and Android 10+ (decision #16), with the mobile navigation from the mockups.

## Scope

- Tauri iOS + Android targets building in CI (iOS on a macOS runner).
- Mobile layout: tab bar (Library · Albums · Shared · Devices), navbar patterns, bottom sheets, safe areas.
- Settings reached from the avatar; the Shared tab shows a placeholder until M5-08.

## Acceptance criteria

- [ ] The app runs on an iPhone (iOS 16) and a Pixel-class Android 10 device and shows a library synced from desktop.

---
_Depends on: M3-17 · Estimate: 3d · Features: —_
