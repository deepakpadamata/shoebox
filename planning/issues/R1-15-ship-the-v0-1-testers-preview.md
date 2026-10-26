---
id: R1-15
title: Ship the v0.1 testers' preview
milestone: R1 · v0.1 testers' preview
labels: [area:infra, type:chore, needs:owner]
estimate_days: 1
depends_on: [R1-03, R1-04, R1-05, R1-06, R1-07, R1-14]
features: []
---

# R1-15 · Ship the v0.1 testers' preview

## Why

Put the app in the hands of a few people who will tell us what breaks: the first wave of the closed beta (M6-08).

## Scope

- Tag `v0.1.0`, release notes from the changelog, the update feed live.
- A one-page tester guide: what to install where, what to expect, known limits (no iOS, keeper tested on x86_64 Linux only,
  about 500–700 MB of memory per 100k photos on a phone, HEIC not imported, how to report a problem with a diagnostics report).
- **Owner:** decide who the testers are (at least one with a keeper) and the feedback channel.

## Acceptance criteria

- [ ] v0.1.0 artifacts published from the tag.
- [ ] Testers have installed it and the feedback channel has its first report.

---
_Depends on: R1-03, R1-04, R1-05, R1-06, R1-07, R1-14 · Estimate: 1d · Features: —_
