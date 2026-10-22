---
id: M1-12
title: Role-based blob fetcher
milestone: M1 · Sync core
labels: [area:node, type:feature]
estimate_days: 3
depends_on: [M1-11, M1-09]
features: [F3.2, F4.3, F4.4]
---

# M1-12 · Role-based blob fetcher

## Why

Turns roles into bytes: Keepers get every original, Phones get previews plus their own originals, Viewers get previews.

## Scope

- `core::policy::fetch_plan(role, record, pins, album_flags)` (pure).
- Fetch queue in priority order: thumbnails → previews → originals; newest first.
- Holder selection: holdings ∩ connected peers, ranked LAN > direct > relay.
- Resumable, verified range downloads; concurrency limits; pause/resume API (used later by sync conditions).
- On-demand `fetch_now(photo, kind)` for the viewer, with progress events naming the source device.

## Acceptance criteria

- [ ] A keeper node ends with every original; a viewer node never downloads an original (testkit).
- [ ] Killing a transfer mid-way resumes without re-downloading completed ranges.

---
_Depends on: M1-11, M1-09 · Estimate: 3d · Features: F3.2, F4.3, F4.4_
