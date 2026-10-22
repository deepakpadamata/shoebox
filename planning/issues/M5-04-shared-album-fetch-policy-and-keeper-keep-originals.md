---
id: M5-04
title: Shared album fetch policy and keeper keep-originals
milestone: M5 · Shared albums
labels: [area:node, area:keeper, type:feature]
estimate_days: 2
depends_on: [M5-02]
features: [F6.9]
---

# M5-04 · Shared album fetch policy and keeper keep-originals

## Why

Decision #5: previews by default; opt in per album to keep every member's originals on my keeper.

## Scope

- Per-album `keep_originals` flag stored per device; the fetcher honours it.
- Keeper CLI `album list|keep|drop`; `status` album rows (mockup 5i).
- Shared-album references block blob GC (hook from M1-13).

## Acceptance criteria

- [ ] Turning keep on for a 1,288-item album makes the keeper fetch all its originals; turning it off frees them on the next GC (unless the library also references them).

---
_Depends on: M5-02 · Estimate: 2d · Features: F6.9_
