---
id: M1-09
title: LAN discovery, relay config, connection reporting
milestone: M1 · Sync core
labels: [area:node, type:feature]
estimate_days: 1.5
depends_on: [M1-01]
features: [F4.7, F8.2, F8.4]
---

# M1-09 · LAN discovery, relay config, connection reporting

## Why

Sync at home without the internet, and honest reporting of direct vs relay paths (mockups 5a/5c).

## Scope

- Enable local (mDNS) discovery, with a config toggle.
- Relay mode: default n0 public relays or a custom URL.
- `peer_info(id)`: path type (LAN/direct/relay + relay name), latency, last seen.

## Acceptance criteria

- [ ] With the internet unplugged, two LAN devices still sync.
- [ ] `peer_info` changes from relay to direct after hole-punching succeeds (manual test, logged).

---
_Depends on: M1-01 · Estimate: 1.5d · Features: F4.7, F8.2, F8.4_
