---
id: M3-16
title: Sync status, tray icon and banners
milestone: M3 · Desktop app
labels: [area:ui, type:feature]
estimate_days: 1.5
depends_on: [M3-13]
features: [F4.4]
---

# M3-16 · Sync status, tray icon and banners

## Why

Users should see that sync is working without opening the Devices view.

## Scope

- Toolbar sync chip ('Receiving 34 from Pixel 9'), tray/menu-bar icon with status and pause/resume.
- Banner component wired to the priority rule (paused > at-risk > recovery).

## Acceptance criteria

- [ ] Pausing from the tray stops transfers within 2 s and the UI reflects it everywhere.

---
_Depends on: M3-13 · Estimate: 1.5d · Features: F4.4_
