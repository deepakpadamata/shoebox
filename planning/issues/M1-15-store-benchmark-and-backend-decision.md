---
id: M1-15
title: Store benchmark and backend decision
milestone: M1 · Sync core
labels: [area:store, type:spike]
estimate_days: 2
depends_on: [M1-05, M1-14]
features: []
---

# M1-15 · Store benchmark and backend decision

## Why

Decision gate for #7: commit to a metadata backend before building UI on top of it.

## Scope

- `tools/store-bench`: 100k and 500k photo libraries; measures initial sync time, incremental sync (100 edits), disk size, peak memory, `photos_by_time` and `at_risk` latency.
- Run on desktop and on a phone-class device (an older iPhone or a low-end Android via a test harness, or an equivalent Raspberry Pi as a proxy).
- Write the decision into `docs/04-open-questions.md` #7.

## Acceptance criteria

- [ ] A results table is committed with the raw numbers.
- [ ] The decision is recorded; if Loro wins, a follow-up issue "Switch default backend to Loro" is created before M2 and the plan is re-estimated.

---
_Depends on: M1-05, M1-14 · Estimate: 2d · Features: —_
