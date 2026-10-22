---
id: M4-09
title: Background sync: Android
milestone: M4 · Mobile apps
labels: [area:mobile, area:android, type:feature]
estimate_days: 2
depends_on: [M4-05, M4-02]
features: [F4.4]
---

# M4-09 · Background sync: Android

## Why

The Android half of best-effort background sync.

## Scope

- WorkManager periodic + expedited work with network/charging constraints, calling `sync_burst`.
- A foreground service with a notification during large transfers (if the user enables it).

## Acceptance criteria

- [ ] Backs up new photos within 1 hour on Wi-Fi without opening the app (Pixel, Samsung test devices).

---
_Depends on: M4-05, M4-02 · Estimate: 2d · Features: F4.4_
