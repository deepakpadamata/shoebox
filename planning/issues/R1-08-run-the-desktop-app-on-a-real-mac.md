---
id: R1-08
title: Run the desktop app on a real Mac
milestone: R1 · v0.1 testers' preview
labels: [area:app, type:test, needs:owner]
estimate_days: 1
depends_on: [R1-01]
features: []
---

# R1-08 · Run the desktop app on a real Mac

## Why

The Tauri window has only been compiled; everything else was checked in a browser against the dev bridge (A-069).
Rendering in WKWebView, the `shoebox://` scheme, Keychain prompts and the tray have never been seen.

## Scope

- `cargo tauri dev` / a release build on macOS, with a fresh library and with an existing one.
- Checklist: window and dark mode, timeline and viewer through `shoebox://`, video playback and posters in WKWebView (A-074),
  Keychain prompt and a restart keeping the device (A-070), tray and close-to-tray, second launch refused, file pickers,
  pairing with a phone, removal with the phrase.
- Fix what breaks; record anything left as an assumption.

## Acceptance criteria

- [ ] Every checklist item ticked or filed as its own issue.
- [ ] Screenshots of the main screens attached to this issue.

## Notes

Needs the owner's Mac (or a Mac the agent can drive).

---
_Depends on: R1-01 · Estimate: 1d · Features: —_
