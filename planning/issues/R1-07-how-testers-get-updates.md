---
id: R1-07
title: How testers get updates
milestone: R1 · v0.1 testers' preview
labels: [area:infra, type:feature]
estimate_days: 1
depends_on: []
features: []
---

# R1-07 · How testers get updates

## Why

The updater plugin was never added (M3-17). Testers need fixes during the preview without guessing when to
reinstall. The update feed itself is published with the release (R1-14), so this is built and tested against a local feed.

## Scope

- Decide: Tauri's signed updater (its own key pair, made now) or a "new version available" banner driven by the site's
  `release.json` with a manual install.
- Implement it for desktop; Android shows the banner (no store yet); the keeper prints a notice in `status`.
- Test against a feed served locally; the real URL is filled in when the repository exists (R1-13).

## Acceptance criteria

- [ ] A 0.1.0 build learns about a 0.1.1 on a local feed and says so (or updates itself).

---
_Depends on: — · Estimate: 1d · Features: —_
