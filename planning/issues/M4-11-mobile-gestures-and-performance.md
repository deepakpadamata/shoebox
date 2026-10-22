---
id: M4-11
title: Mobile gestures and performance
milestone: M4 · Mobile apps
labels: [area:mobile, area:ui, type:feature]
estimate_days: 3
depends_on: [M4-07]
features: [F2.3, F3.1, F2.9]
---

# M4-11 · Mobile gestures and performance

## Why

It has to feel like a native photos app on a phone.

## Scope

- Pinch to change grid density, swipe between photos, pinch-zoom in the viewer, swipe down to dismiss.
- Multi-select by drag; long-press menus (mockup 2b).
- Memory budget: thumbnail cache limits; tested with a 100k library on a 4 GB Android device.
- Video playback on mobile.

## Acceptance criteria

- [ ] Scrolling a 100k library stays above 50 fps on an iPhone 11 / Pixel 6-class device.
- [ ] No out-of-memory kill during 10 minutes of fast scrolling.

---
_Depends on: M4-07 · Estimate: 3d · Features: F2.3, F3.1, F2.9_
