---
id: M4-07
title: Mobile onboarding, QR scan and join
milestone: M4 · Mobile apps
labels: [area:mobile, area:ui, type:feature]
estimate_days: 3
depends_on: [M4-01, M3-07]
features: [F1.2, F1.3, F1.4, F1.5, F4.1]
---

# M4-07 · Mobile onboarding, QR scan and join

## Why

Mockups 1a–1d on phones, including scanning the desktop's QR.

## Scope

- QR scanning (existing Tauri barcode-scanner plugin if suitable, otherwise added to the photo-library native code).
- Welcome → new library or join (scan + SAS confirm) → name + recovery kit (save to Files / share sheet) → backup + role (default Phone).
- Admin cert issued to the first phone (decision #18).

## Acceptance criteria

- [ ] A phone joins a desktop library by scanning, with the SAS shown on both screens.
- [ ] A phone can also start a new library and later pair a desktop to it.

---
_Depends on: M4-01, M3-07 · Estimate: 3d · Features: F1.2, F1.3, F1.4, F1.5, F4.1_
