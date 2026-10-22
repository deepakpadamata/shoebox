---
id: M1-08
title: Library sync between nodes
milestone: M1 · Sync core
labels: [area:node, type:feature]
estimate_days: 2
depends_on: [M1-05, M1-02]
features: []
---

# M1-08 · Library sync between nodes

## Why

Metadata replication: any device with the library capability converges on the same library.

## Scope

- Library ticket (dev-only for now, replaced by pairing in M2) to share the capability.
- Automatic sync on connect and on gossip nudge; backoff on failure.
- Sync progress `NodeEvent`s per peer.

## Acceptance criteria

- [ ] Two desktops on different networks (one via relay) converge 10k records in under 60 s.
- [ ] Edits made while offline converge after reconnect (testkit).

---
_Depends on: M1-05, M1-02 · Estimate: 2d · Features: —_
