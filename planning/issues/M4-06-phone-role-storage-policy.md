---
id: M4-06
title: Phone role storage policy
milestone: M4 · Mobile apps
labels: [area:mobile, area:core, type:feature]
estimate_days: 2
depends_on: [M4-05, M2-08]
features: [F4.3, F7.3]
---

# M4-06 · Phone role storage policy

## Why

Decision #3: previews of everything, originals of its own captures until they're safe elsewhere.

## Scope

- `fetch_plan` and retention for Phone: own captures kept until copy_count ≥ target; then they're eligible for free-up-space (never auto-deleted without user action in MVP).
- Pinned items always kept.

## Acceptance criteria

- [ ] Testkit: phone + keeper → after the keeper confirms, the phone's own originals show as freeable; before that they don't.

---
_Depends on: M4-05, M2-08 · Estimate: 2d · Features: F4.3, F7.3_
