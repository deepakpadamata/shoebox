---
id: R1-01
title: Version 0.1.0 and the format and protocol baseline
milestone: R1 · v0.1 testers' preview
labels: [area:core, type:chore]
estimate_days: 1
depends_on: []
features: []
---

# R1-01 · Version 0.1.0 and the format and protocol baseline

## Why

Until now every wire message and on-disk file could change freely because nothing was released (A-084).
Once testers install v0.1, a change that an older build cannot read loses their data or splits their devices.
v0.1 has to be the line from which compatibility counts.

## Scope

- Bump every crate, the Tauri app (`tauri.conf.json`), the UI `package.json` and the keeper to 0.1.0.
- Declare the current `format.json` layout as the v0.1 baseline, and make sure an empty marker-less data dir and a dev
  library from before today are adopted or refused clearly (not half-read).
- List every protocol name (`shoebox/*/N`, iroh ALPNs) and every on-disk file with its version in one place
  (`docs/notes/formats.md`); a test fails when a protocol name or file version changes without that list changing.
- Write the rule into `CONTRIBUTING.md`: from 0.1 on, a release serves the previous version of each protocol it changes,
  and a layout change ships with a migration (`shoebox_node::format`).

## Acceptance criteria

- [ ] `cargo run --bin desktop-dev-server -- --version` (and the keeper) print 0.1.0.
- [ ] The formats list exists and the guarding test fails when a protocol name changes.
- [ ] A data dir written by the 0.1.0 build opens in a later build of the same layout; a newer layout is refused with a message.

## Notes

Settles A-084 (2) for real use. Today's changes (epoch notices, presence/2, admins-seen) are part of the baseline.

---
_Depends on: — · Estimate: 1d · Features: —_
