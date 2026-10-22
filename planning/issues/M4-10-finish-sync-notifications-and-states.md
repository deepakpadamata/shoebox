---
id: M4-10
title: Finish-sync notifications and states
milestone: M4 · Mobile apps
labels: [area:mobile, area:ui, type:feature]
estimate_days: 2.5
depends_on: [M4-08, M4-09]
features: [F4.4, F7.1]
---

# M4-10 · Finish-sync notifications and states

## Why

Mockups 5d–5f and the rules in design/README.

## Scope

- Local notification when: at-risk photos exist, a peer is reachable, and the background budget ran out. At most once a day, with an ETA.
- Foreground 'Finishing backup…' card with ETA and keep-awake toggle (keep-awake plugin).
- 'Backup paused: none of your devices are reachable' banner; no notification in that case.

## Acceptance criteria

- [ ] Each rule is covered by unit tests on the decision function.
- [ ] Manual test matrix (reachable / unreachable × budget left / exhausted) passes on iOS and Android.

---
_Depends on: M4-08, M4-09 · Estimate: 2.5d · Features: F4.4, F7.1_
