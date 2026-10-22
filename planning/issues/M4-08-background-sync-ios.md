---
id: M4-08
title: Background sync: iOS
milestone: M4 · Mobile apps
labels: [area:mobile, area:ios, type:feature]
estimate_days: 3
depends_on: [M4-05, M4-02]
features: [F4.4]
---

# M4-08 · Background sync: iOS

## Why

Decision #10: best-effort background sync using what iOS allows.

## Scope

- `BGProcessingTask` (requires power, network) and `BGAppRefreshTask` registration via the bg-sync plugin.
- `Node::sync_burst(deadline)`: connect to known peers (keeper first), push at-risk originals first, then metadata; stop cleanly before the deadline.
- Record outcomes (bytes, peers reached, time used) for the notification rules and diagnostics.

## Acceptance criteria

- [ ] Overnight on charger + Wi-Fi with a keeper reachable: 500 new photos back up without opening the app (measured on a real device, results logged).
- [ ] No crash or watchdog kill in 20 consecutive background runs.

---
_Depends on: M4-05, M4-02 · Estimate: 3d · Features: F4.4_
