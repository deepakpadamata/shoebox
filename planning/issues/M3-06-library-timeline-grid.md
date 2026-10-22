---
id: M3-06
title: Library timeline grid
milestone: M3 · Desktop app
labels: [area:ui, type:feature]
estimate_days: 4
depends_on: [M3-03, M3-04, M3-05]
features: [F2.3, F2.6]
---

# M3-06 · Library timeline grid

## Why

The everyday view (mockups 2a, 2c). It has to stay smooth at 100k photos.

## Scope

- Virtualized grid grouped by day/month/year with a density switch; sticky date headers with place names when available.
- Badges only on at-risk ('1 copy') and syncing items (decision #13).
- Sidebar: library sections, albums, shared, live device roster with path dots.
- Drag-and-drop import onto the window; import progress.

## Acceptance criteria

- [ ] 60 fps scrolling through a 100k library on a 2020-era laptop.
- [ ] Badges update live as copies land on other devices.

---
_Depends on: M3-03, M3-04, M3-05 · Estimate: 4d · Features: F2.3, F2.6_
