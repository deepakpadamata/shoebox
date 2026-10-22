---
id: M3-02
title: Platform keystore for device secrets
milestone: M3 · Desktop app
labels: [area:app, area:security, type:feature]
estimate_days: 1
depends_on: [M3-01]
features: [F1.1]
---

# M3-02 · Platform keystore for device secrets

## Why

F1.1: keep the endpoint secret (and an optionally retained root key) in the OS keystore, not in a plain file.

## Scope

- Use the `keyring` crate: macOS Keychain, Windows Credential Manager, Secret Service on Linux (fall back to a file with a warning).
- Migrate the dev file-based key into the keystore on first launch.

## Acceptance criteria

- [ ] After migration there's no secret material in the app data dir (verified on all three desktop OSes).

---
_Depends on: M3-01 · Estimate: 1d · Features: F1.1_
