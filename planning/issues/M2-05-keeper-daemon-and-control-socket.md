---
id: M2-05
title: Keeper daemon and control socket
milestone: M2 · Identity, pairing & keeper
labels: [area:keeper, type:feature]
estimate_days: 2
depends_on: [M1-12, M2-04]
features: [F4.6]
---

# M2-05 · Keeper daemon and control socket

## Why

`shoebox-keeper run` is the always-on backstop (decision #4).

## Scope

- `run`: open the node with role Keeper, start the fetcher, handle signals, structured logs (journald-friendly).
- Local control socket (Unix domain socket, `0600`) exposing a small RPC used by the other subcommands.
- Data dir from `--storage` / `$XDG_DATA_HOME/shoebox`; endpoint secret in a `0600` file.

## Acceptance criteria

- [ ] Keeper survives restart mid-fetch and resumes.
- [ ] A second `run` on the same data dir refuses to start (lock file).

---
_Depends on: M1-12, M2-04 · Estimate: 2d · Features: F4.6_
