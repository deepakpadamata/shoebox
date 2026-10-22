---
id: M2-03
title: Enforce device list and revocation
milestone: M2 · Identity, pairing & keeper
labels: [area:node, area:security, type:feature]
estimate_days: 3
depends_on: [M2-01]
features: [F4.5]
---

# M2-03 · Enforce device list and revocation

## Why

A revoked or unknown device must not be able to read or write the library from then on.

## Scope

- Accept connections and sync only from endpoints in the valid device list.
- Ignore and refuse writes authored by revoked devices after the revocation HLC.
- If the backend's capability is a bearer secret (iroh-docs namespace), revocation rotates it: create a new namespace, migrate, and hand the new capability to the remaining devices over authenticated connections.
- Revoked device: shows a 'removed from library' state and stops syncing.

## Acceptance criteria

- [ ] Testkit: revoke node C → C can no longer fetch new records or blobs; A and B keep syncing after the rotation.
- [ ] A rotation with 100k records completes without data loss.

## Notes

This is the most complex M2 issue. If it overruns, split rotation into its own issue.

---
_Depends on: M2-01 · Estimate: 3d · Features: F4.5_
