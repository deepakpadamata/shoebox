---
id: M3-01
title: Tauri 2 desktop app scaffold
milestone: M3 · Desktop app
labels: [area:app, type:chore]
estimate_days: 2
depends_on: [M2-06]
features: []
---

# M3-01 · Tauri 2 desktop app scaffold

## Why

The shell that hosts `Node` in-process and renders the Svelte UI (decisions #11 and #15).

## Scope

- `apps/desktop-mobile`: Tauri 2 + Svelte 5 + Vite + TypeScript; app data dir; single-instance lock.
- `Node` started on app launch on Tauri's Tokio runtime; clean shutdown on quit.
- Desktop window layout shell: sidebar + main + optional inspector (mockup 2c).

## Acceptance criteria

- [ ] `pnpm tauri dev` runs on macOS, Windows and Linux.
- [ ] The app opens an existing dev library created with `shoebox-dev`.

---
_Depends on: M2-06 · Estimate: 2d · Features: —_
