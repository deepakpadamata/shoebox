---
id: M2-03b
title: Rotate the library capability on revocation
milestone: M2 · Identity, pairing & keeper
labels: [area:node, area:security, type:feature]
estimate_days: 4
depends_on: [M2-03, M2-04]
features: [F4.5]
---

# M2-03b · Rotate the library capability on revocation

Split out of M2-03 (which allowed this: "if it overruns, split rotation into its own issue").

## Why

M2-03 refuses connections from revoked devices, but a revoked device still *holds* the bearer capability (the iroh-docs namespace secret). It
cannot reach any member, yet the secret is not revoked. Rotation makes revocation survive a revoked device that later shows up on a path we
did not gate, and is required before an album/library capability is considered confidential.

## Scope

- On revoking a device, an admin creates a new library space (new namespace, same root key), copies every valid entry into it (`LibraryState::snapshot_entries`)
  and hands the new capability to every remaining device over an authenticated connection (new ALPN `shoebox/cap/0`; the receiver checks the sender is an admin in its own
  device list and that the revocation cert is valid).
- Members switch their `LibraryNode` to the new store (needs `LibraryNode` to hold a swappable store) and drop the old namespace.
- Offline members get the capability on next contact; until then they keep working on the old space, which only members can reach anyway.

## Acceptance criteria

- [ ] Testkit: after revoking C and rotating, A and B keep syncing on the new space; C's old capability opens nothing reachable.
- [ ] A rotation with 100k records completes without data loss (Loro or docs backend, whichever M1-16 settles on).
