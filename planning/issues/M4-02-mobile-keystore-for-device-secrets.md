---
id: M4-02
title: Mobile keystore for device secrets
milestone: M4 · Mobile apps
labels: [area:mobile, area:security, type:feature]
estimate_days: 1
depends_on: [M4-01]
features: [F1.1]
---

# M4-02 · Mobile keystore for device secrets

## Why

The mobile equivalent of M3-02.

## Scope

- iOS Keychain (`kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`, so background tasks can read it); Android Keystore-wrapped key.
- Excluded from cloud backups so the identity doesn't silently clone onto another phone.

## Acceptance criteria

- [ ] Background tasks can open the node while the phone is locked (after first unlock).
- [ ] Restoring an iOS backup to a new phone does not reuse the old endpoint key.

---
_Depends on: M4-01 · Estimate: 1d · Features: F1.1_
