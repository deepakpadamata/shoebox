---
id: R1-05
title: About screen, licences and source offer
milestone: R1 · v0.1 testers' preview
labels: [area:app, type:chore]
estimate_days: 1
depends_on: [R1-01]
features: []
---

# R1-05 · About screen, licences and source offer

## Why

The apps are AGPL-3.0 and bundle many third-party crates and npm packages. Handing builds to people requires the
licence texts, the attributions (e.g. IJG for JPEG code if it ships) and an offer of the source.

## Scope

- Settings → About: version, build commit, AGPL notice, link to the source repository.
- A generated third-party licence list (cargo-about or similar for Rust, the npm equivalent for the UI) bundled with each app
  and the keeper (`shoebox-keeper licences`).
- `cargo deny` stays the guard for which licences are allowed.

## Acceptance criteria

- [ ] About shows 0.1.0 and the commit; licences open in the app.
- [ ] The keeper prints its licences.
- [ ] CI regenerates the list and fails if it is stale.

---
_Depends on: R1-01 · Estimate: 1d · Features: —_
