---
id: M6-01
title: Real-network nightly test
milestone: M6 · Hardening & beta
labels: [area:infra, type:test]
estimate_days: 2
depends_on: [M5-08]
features: []
---

# M6-01 · Real-network nightly test

## Why

Hole punching and relay fallback only show up on real networks.

## Scope

- A keeper on a VPS and a CI runner (or home box) behind NAT; a nightly scripted scenario: pair, import 500, sync, fetch, revoke.
- Report: path types, durations, failures; alert on regression.

## Acceptance criteria

- [ ] The job runs nightly for 2 weeks with results archived.

---
_Depends on: M5-08 · Estimate: 2d · Features: —_
