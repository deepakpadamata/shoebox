---
id: M3-10
title: Video posters and playback
milestone: M3 · Desktop app
labels: [area:media, area:ui, type:feature]
estimate_days: 3
depends_on: [M3-04, M3-08]
features: [F2.9]
---

# M3-10 · Video posters and playback

## Why

Videos are in the MVP: thumbnails and previews need a poster frame, and playback must stream.

## Scope

- Poster frame + duration extraction on desktop/keeper (ffmpeg sidecar, optional dependency; fall back to a generic tile).
- Video tiles with duration; viewer playback via `<video>` over the blob URI with range requests.
- Originals stream from a peer while playing (fetch ahead).

## Acceptance criteria

- [ ] A 1 GB MOV held only on a LAN peer starts playing within 3 s.
- [ ] Without ffmpeg, videos still import and play, with a placeholder poster.

---
_Depends on: M3-04, M3-08 · Estimate: 3d · Features: F2.9_
