---
id: M2-01
title: Identity: root key, admin certs, signed device list
milestone: M2 · Identity, pairing & keeper
labels: [area:core, area:security, type:feature]
estimate_days: 3
depends_on: [M1-15]
features: [F1.1, F4.5]
---

# M2-01 · Identity: root key, admin certs, signed device list

## Why

Decisions #9 and #18: per-device keys, a library root key, and admin certs that let chosen devices pair and revoke.

## Scope

- `RootKey` (ed25519) generation; `AdminCert{device, issued_at, expires?}` signed by root; `DeviceCert{device, role, name, added_by, hlc}` signed by an admin.
- `SignedDeviceList` validation: chain root → admin → device; revocations (`RevokeCert`) signed by an admin or the root.
- On library creation: generate root, issue admin certs to the creating device, and to the first device of the other form factor when it pairs (first phone / first desktop).
- Wire up the signature checks in `apply_device_change` (stubbed in M0-06).

## Acceptance criteria

- [ ] Unit tests: valid chain accepted; forged, expired and revoked certs rejected; revocation by a non-admin rejected.
- [ ] The root private key is never written to disk except when the recovery kit is exported (or explicitly kept on the first device).

---
_Depends on: M1-15 · Estimate: 3d · Features: F1.1, F4.5_
