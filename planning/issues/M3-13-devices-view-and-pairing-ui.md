---
id: M3-13
title: Devices view and pairing UI
milestone: M3 · Desktop app
labels: [area:ui, type:feature]
estimate_days: 3
depends_on: [M3-06, M2-04, M2-03]
features: [F4.1, F4.2, F4.3, F4.5, F8.4]
---

# M3-13 · Devices view and pairing UI

## Why

Mockups 5a–5c and the desktop half of 5h.

## Scope

- Device list: protection summary, role, path dot, storage, last seen; activity section.
- Add device: QR + text ticket (`shoebox-keeper pair …`), countdown, SAS confirm sheet with what the device will be able to do.
- Device detail: role change, storage, connection diagnostics, endpoint id, remove device (admin only).

## Acceptance criteria

- [ ] A phone (M4) or keeper pairs against the desktop using the UI only.
- [ ] Removing a device triggers revocation and shows its progress.

---
_Depends on: M3-06, M2-04, M2-03 · Estimate: 3d · Features: F4.1, F4.2, F4.3, F4.5, F8.4_
