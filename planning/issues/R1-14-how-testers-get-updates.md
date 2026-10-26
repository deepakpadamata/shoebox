---
id: R1-14
title: How testers get updates
milestone: R1 · v0.1 testers' preview
labels: [area:infra, type:feature]
estimate_days: 1
depends_on: [R1-13]
features: []
---

# R1-14 · How testers get updates

## Why

The updater plugin was never added (M3-17). Testers need fixes during the preview without guessing when to reinstall.

## Scope

- Decide: Tauri's signed updater feed, or a "new version available" banner driven by the site's `release.json`
  with manual install.
- Implement the choice for desktop; Android shows the banner (no store yet); the keeper prints a notice in `status`.

## Acceptance criteria

- [ ] A build of 0.1.0 learns about a 0.1.1 published to the feed and says so (or updates itself).

---
_Depends on: R1-13 · Estimate: 1d · Features: —_
