---
id: M3-14
title: Storage health and free up space
milestone: M3 · Desktop app
labels: [area:ui, type:feature]
estimate_days: 2
depends_on: [M3-06, M2-08]
features: [F7.1, F7.2, F7.3, F7.4]
---

# M3-14 · Storage health and free up space

## Why

Mockup 6a: copy-count buckets, redundancy target, safe free-up.

## Scope

- Health card with 3+/2/1 copy buckets and a 'View' link to at-risk items.
- Redundancy target setting.
- Per-device storage breakdown (originals/previews/thumbnails) and 'Free up X GB' with an explanation and a dry-run figure.

## Acceptance criteria

- [ ] Displayed numbers match `at_risk` / `safe_to_free` exactly (test with fixtures).

---
_Depends on: M3-06, M2-08 · Estimate: 2d · Features: F7.1, F7.2, F7.3, F7.4_
