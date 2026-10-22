---
id: M3-03
title: Typed command and event bridge
milestone: M3 · Desktop app
labels: [area:app, type:feature]
estimate_days: 2
depends_on: [M3-01]
features: []
---

# M3-03 · Typed command and event bridge

## Why

A typed boundary between Rust and Svelte keeps the UI honest as the API grows.

## Scope

- Tauri commands for the `Node` API surface listed in 05 §6, with TS bindings generated via `tauri-specta`.
- A single event channel forwarding `Change`/`NodeEvent`; Svelte stores subscribe to it.
- Error type mapped to user-facing messages.

## Acceptance criteria

- [ ] Bindings are generated in CI; a type mismatch fails the build.
- [ ] The UI updates live when another device adds a photo.

---
_Depends on: M3-01 · Estimate: 2d · Features: —_
