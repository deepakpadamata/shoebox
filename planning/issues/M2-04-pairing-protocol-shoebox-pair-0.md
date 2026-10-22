---
id: M2-04
title: Pairing protocol shoebox/pair/0
milestone: M2 · Identity, pairing & keeper
labels: [area:node, area:security, type:feature]
estimate_days: 3
depends_on: [M2-01, M1-09]
features: [F1.2, F4.1]
---

# M2-04 · Pairing protocol shoebox/pair/0

## Why

Adding a device = QR or ticket + a 6-digit code check (mockups 1c, 5b, 5g, 5h).

## Scope

- `PairTicket{addr, secret, expires}` encode/decode (string + QR payload).
- ALPN handler: HMAC proof of the secret, SAS derivation, a confirm round-trip, an admin-signed `DeviceCert`, library capability handoff.
- Single-use and expiring tickets (5 min default); rate-limit failed proofs.
- Events: `PairingCode{sas, peer_name, peer_role}`, `PairingDone`, `PairingFailed{reason}`.
- Both directions: a camera device scans the QR, or a headless device dials a pasted ticket.

## Acceptance criteria

- [ ] Pairing works LAN-only and through a relay.
- [ ] A wrong SAS confirmation aborts with no state change on either side.
- [ ] Reusing a ticket fails.

---
_Depends on: M2-01, M1-09 · Estimate: 3d · Features: F1.2, F4.1_
