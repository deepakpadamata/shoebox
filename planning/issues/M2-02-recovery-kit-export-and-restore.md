---
id: M2-02
title: Recovery kit export and restore
milestone: M2 · Identity, pairing & keeper
labels: [area:core, area:security, type:feature]
estimate_days: 2
depends_on: [M2-01]
features: [F1.4]
---

# M2-02 · Recovery kit export and restore

## Why

With no account, the recovery kit is the only way back if every device is lost.

## Scope

- Encode the root key as a 24-word phrase (BIP39 wordlist) plus a printable/PDF-ready text file with library id and creation date.
- `Node::export_recovery_kit()` and `Node::restore_from_kit(phrase)`: rejoin as a new admin device, re-sign itself, sync from any surviving holder (incl. keeper).
- `recovery_kit_saved` flag in the library (drives the nag banner, mockup 2d).

## Acceptance criteria

- [ ] Restore on a fresh machine rejoins the library and sees all photos once a keeper is reachable.
- [ ] A wrong phrase fails with a clear error and no partial state.

---
_Depends on: M2-01 · Estimate: 2d · Features: F1.4_
