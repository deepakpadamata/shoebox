---
id: M2-06
title: Keeper CLI commands
milestone: M2 · Identity, pairing & keeper
labels: [area:keeper, type:feature]
estimate_days: 2
depends_on: [M2-05]
features: [F4.2, F4.6, F8.4]
---

# M2-06 · Keeper CLI commands

## Why

The keeper's whole UI (mockups 5g, 5i).

## Scope

- `pair <ticket> [--name] [--storage] [--role]` with the terminal output of mockup 5g.
- `status [--json]` (mockup 5i): devices, paths, library counts, at-risk, shared-album policy (album rows filled in M5-04).
- `devices [revoke <id>]` (needs an admin cert).
- `album list|keep|drop` stubs that return 'available after shared albums' until M5-04.

## Acceptance criteria

- [ ] Output matches the mockups' content; `--json` output has a documented schema.
- [ ] Every command works against a running daemon over the control socket.

---
_Depends on: M2-05 · Estimate: 2d · Features: F4.2, F4.6, F8.4_
