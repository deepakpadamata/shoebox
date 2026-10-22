---
id: M3-17
title: Desktop packaging and auto-update
milestone: M3 · Desktop app
labels: [area:app, area:infra, type:chore]
estimate_days: 3
depends_on: [M3-16]
features: []
---

# M3-17 · Desktop packaging and auto-update

## Why

Signed installers so beta testers can install without warnings.

## Scope

- macOS: signed and notarized DMG (universal). Windows: signed MSI/NSIS. Linux: AppImage + .deb.
- Tauri updater with a signed update feed (GitHub Releases).
- Release workflow in CI.

## Acceptance criteria

- [ ] Clean installs on macOS 12, Windows 10 and Ubuntu 22.04 launch without security prompts beyond the OS defaults.
- [ ] An update from N to N+1 applies and keeps the library intact.

---
_Depends on: M3-16 · Estimate: 3d · Features: —_
