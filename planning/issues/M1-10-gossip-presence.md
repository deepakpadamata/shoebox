---
id: M1-10
title: Gossip presence
milestone: M1 · Sync core
labels: [area:node, type:feature]
estimate_days: 1.5
depends_on: [M1-08]
features: [F4.2]
---

# M1-10 · Gossip presence

## Why

The UI shows who's online and reacts quickly to new photos, without polling.

## Scope

- One gossip topic per library (and later per album) derived from the namespace id.
- Messages: `Hello{name, role, app_version}`, `Heartbeat`, `NewItems{count}` (a nudge to sync).
- Online / last-seen table exposed via `Node::devices()`.

## Acceptance criteria

- [ ] A device going offline shows as offline within 60 s.
- [ ] `NewItems` triggers sync on peers within 2 s on LAN.

---
_Depends on: M1-08 · Estimate: 1.5d · Features: F4.2_
