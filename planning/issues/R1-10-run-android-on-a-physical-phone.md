---
id: R1-10
title: Run Android on a physical phone
milestone: R1 · v0.1 testers' preview
labels: [area:android, type:test, needs:owner]
estimate_days: 1
depends_on: [R1-01]
features: []
---

# R1-10 · Run Android on a physical phone

## Why

Android was only checked on an emulator: the QR scan never ran (A-077), notifications were never seen (A-076),
background backup timing is up to the OS.

## Scope

- Release-mode APK on a real phone (any Android 10+).
- Checklist: join by QR, camera-roll backup with Wi-Fi/charging rules, backup with the app closed (WorkManager), a notification,
  keystore surviving a restart, gestures, a 5k-photo library's memory use.

## Acceptance criteria

- [ ] Checklist ticked or filed.
- [ ] Memory use at 5k photos recorded in A-066's note.

---
_Depends on: R1-01 · Estimate: 1d · Features: —_
