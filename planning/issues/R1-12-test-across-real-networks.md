---
id: R1-12
title: Test across real networks
milestone: R1 · v0.1 testers' preview
labels: [area:node, type:test, needs:owner]
estimate_days: 2
depends_on: [R1-08]
features: []
---

# R1-12 · Test across real networks

## Why

Sync, pairing and recovery have only run on one machine. Relays, hole punching and two home networks have never been
exercised (A-042, A-044, A-062), and for a peer-to-peer app that is the largest unknown.

## Scope

- **Owner:** a small VPS (or two machines on different networks).
- Run `scripts/real-network/scenario.py` against it and enable the nightly workflow (M6-01).
- Manual: two devices on different networks, one behind a strict NAT; pair over relay, sync 1k photos, confirm the path
  changes from relay to direct where possible.

## Acceptance criteria

- [ ] Nightly green for 3 nights.
- [ ] Manual run notes in `docs/notes/real-network-nightly.md` with measured times.

---
_Depends on: R1-08 · Estimate: 2d · Features: —_
