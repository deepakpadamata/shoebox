---
id: R1-11
title: Keeper on a Raspberry Pi
milestone: R1 · v0.1 testers' preview
labels: [area:keeper, type:test, needs:owner]
estimate_days: 1
depends_on: [R1-01]
features: []
---

# R1-11 · Keeper on a Raspberry Pi

## Why

The keeper's service install and the "install, reboot, keeps syncing" path were never run on real hardware (A-063).

## Scope

- Install with `scripts/install-keeper.sh` on Raspberry Pi OS, `service install`, reboot.
- Pair it from the app, fill it with a few hundred photos, check it keeps every original and comes back after a reboot.
- Recovery through the keeper (it lets it through and logs, A-087).

## Acceptance criteria

- [ ] Keeper syncing again within a minute of a reboot.
- [ ] Its status shows the right counts; a recovery through it is logged.

---
_Depends on: R1-01 · Estimate: 1d · Features: —_
