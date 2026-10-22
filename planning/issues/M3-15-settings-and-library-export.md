---
id: M3-15
title: Settings and library export
milestone: M3 · Desktop app
labels: [area:ui, type:feature]
estimate_days: 2.5
depends_on: [M3-06]
features: [F1.3, F8.1, F8.2, F8.3, F8.4, F8.5]
---

# M3-15 · Settings and library export

## Why

Mockups 6b/6c, plus the no-lock-in export.

## Scope

- Profile (name, avatar), recovery kit status, sync conditions (desktop: pause on metered/battery).
- Location when sharing: Ask each time / Always keep / Always remove (used in M5-03).
- Relay servers: default or custom URL, current path per peer; local network discovery toggle.
- Diagnostics: endpoint id, export a logs bundle.
- Export library to folder: originals in `YYYY/MM/` with sidecar JSON (albums, favorites); resumable.

## Acceptance criteria

- [ ] Export of the `medium` fixture library re-imports into a fresh library with identical ids, albums and favorites.

---
_Depends on: M3-06 · Estimate: 2.5d · Features: F1.3, F8.1, F8.2, F8.3, F8.4, F8.5_
