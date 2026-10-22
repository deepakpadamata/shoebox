---
id: M5-03
title: Location prompt and stripped copies
milestone: M5 · Shared albums
labels: [area:media, area:node, type:feature]
estimate_days: 2.5
depends_on: [M5-01, M3-15]
features: [F8.3]
---

# M5-03 · Location prompt and stripped copies

## Why

Decision #8: share originals by default; ask when location is present; stripping creates derived copies.

## Scope

- `strip_location(original) -> derived`: remove GPS EXIF/XMP without re-encoding (JPEG, HEIC, PNG; MP4/MOV location atoms).
- `SharedItem.shared_hash` mapping; dedupe stays intact on the owner's side.
- Setting Ask / Always keep / Always remove; 'Remember for this album'.

## Acceptance criteria

- [ ] Byte-diff tests: only location metadata changes; pixel data is identical.
- [ ] Members never receive the GPS-bearing blob when 'Remove location' is chosen (testkit, network capture).

---
_Depends on: M5-01, M3-15 · Estimate: 2.5d · Features: F8.3_
