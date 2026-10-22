---
id: M2-07
title: Keeper service install
milestone: M2 · Identity, pairing & keeper
labels: [area:keeper, type:chore]
estimate_days: 1
depends_on: [M2-06]
features: [F4.6]
---

# M2-07 · Keeper service install

## Why

Most users will run the keeper on a NAS or Pi and want it to start at boot.

## Scope

- `service install|uninstall`: a systemd unit (Linux) and a launchd plist (macOS), running as the invoking user by default, with an option for a dedicated user.
- Restart policy and log routing.

## Acceptance criteria

- [ ] Fresh Raspberry Pi OS: install → reboot → keeper is running and syncing.
- [ ] Uninstall removes the unit and leaves the data intact.

---
_Depends on: M2-06 · Estimate: 1d · Features: F4.6_
