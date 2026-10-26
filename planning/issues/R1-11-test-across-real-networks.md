---
id: R1-11
title: Test across real networks
milestone: R1 · v0.1 testers' preview
labels: [area:node, type:test, needs:owner]
estimate_days: 2
depends_on: []
features: []
---

# R1-11 · Test across real networks

## Why

Sync, pairing and recovery have only run on one machine. Relays, hole punching and two networks have never been
exercised (A-042, A-044, A-062), and for a peer-to-peer app that is the largest unknown.

## Scope

- **Owner:** two machines on different networks (for example the Mac at home and the Linux box elsewhere, or a phone on
  mobile data), or a small VPS.
- Run `scripts/real-network/scenario.py` between them (pair, import, sync, fetch every original, revoke).
- Manual: pair over the relay, sync 1k photos, confirm the path changes from relay to direct where the networks allow.
- The nightly workflow (M6-01) is switched on with the repository (R1-13).

## Acceptance criteria

- [ ] The scenario script passes across two real networks.
- [ ] Notes in `docs/notes/real-network-nightly.md` with the measured times and paths.

---
_Depends on: — · Estimate: 2d · Features: —_
