---
id: R1-10
title: Keeper on a Linux machine
milestone: R1 · v0.1 testers' preview
labels: [area:keeper, type:test, needs:owner]
estimate_days: 1
depends_on: [R1-01]
features: []
---

# R1-10 · Keeper on a Linux machine

## Why

The keeper's service install and the "install, reboot, keeps syncing" path were never run on real hardware (A-063).
The owner has a Linux box (no Raspberry Pi), so that is the first real keeper; arm64/Pi builds stay unverified for v0.1.

## Scope

- Build the keeper for the box (`cargo build --release -p keeper`, or the release workflow's musl build once R1-14 runs),
  install it with `shoebox-keeper service install` (systemd), reboot.
- Pair it from the app, fill it with a few hundred photos, check it keeps every original and comes back after a reboot.
- Recovery through the keeper (it lets it through and logs, A-087).
- The install script (`scripts/install-keeper.sh`) is checked against real signed downloads in R1-14.

## Acceptance criteria

- [ ] Keeper syncing again within a minute of a reboot.
- [ ] Its status shows the right counts; a recovery through it is logged.

## Notes

Raspberry Pi / arm64 is out of v0.1 testing: no device. Recorded in the tester guide's known limits.

---
_Depends on: R1-01 · Estimate: 1d · Features: —_
