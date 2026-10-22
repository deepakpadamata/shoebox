---
id: M0-05
title: Merge rules (LWW, add-wins set, tombstones)
milestone: M0 · Foundations
labels: [area:core, type:feature]
estimate_days: 1.5
depends_on: [M0-04]
features: [F2.7, F2.8]
---

# M0-05 · Merge rules (LWW, add-wins set, tombstones)

## Why

Conflict semantics are defined once and every backend must reproduce them (05 §2). Bugs here mean data loss, so we test with properties.

## Scope

- `Lww<T>` ordered by HLC with device-id tie-break.
- Add-wins observed-remove set for album membership.
- Tombstone rule: delete beats edits at or before the delete's HLC; restore is an explicit LWW write.
- `merge(a, b)` for `PhotoRecord`, `AlbumMeta`, album sets.

## Acceptance criteria

- [ ] proptest: merge is commutative, associative and idempotent for every mergeable type.
- [ ] Unit tests for the concurrent add/remove case and the delete-vs-favorite race.

---
_Depends on: M0-04 · Estimate: 1.5d · Features: F2.7, F2.8_
