---
id: M3-08
title: Photo viewer with on-demand originals
milestone: M3 · Desktop app
labels: [area:ui, type:feature]
estimate_days: 3
depends_on: [M3-06, M1-12]
features: [F3.1, F3.2]
---

# M3-08 · Photo viewer with on-demand originals

## Why

Mockups 3a and 3c: instant preview, the original streamed from a named peer, honest unavailable state.

## Scope

- Full-window viewer: keyboard and trackpad navigation, zoom/pan.
- Shows the preview immediately and upgrades to the original on zoom or after a dwell; progress chip with source device + path.
- Unavailable state: who has it and when they were last seen; 'Notify me when available'.

## Acceptance criteria

- [ ] Opening a photo whose original lives only on an online LAN peer shows the original in < 2 s for a 30 MB HEIC.
- [ ] With the holder offline, the unavailable state appears within 3 s.

---
_Depends on: M3-06, M1-12 · Estimate: 3d · Features: F3.1, F3.2_
