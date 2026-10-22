---
id: M1-06
title: Folder import with dedupe
milestone: M1 · Sync core
labels: [area:node, area:media, type:feature]
estimate_days: 2
depends_on: [M1-03, M1-04, M1-05]
features: [F2.2, F2.4]
---

# M1-06 · Folder import with dedupe

## Why

Desktop import from folders, the first real path for data to enter the library.

## Scope

- `Node::import_paths(paths)`: walk, analyze, add blobs, upsert `PhotoRecord` with `imported_by`.
- Dedupe by `PhotoId`: an existing id only adds a holding.
- Watched folders via `notify` with debounce; persisted list in config.
- Import progress events.

## Acceptance criteria

- [ ] Importing the same folder twice creates no new records.
- [ ] A file dropped into a watched folder appears in the library within 5 s.

---
_Depends on: M1-03, M1-04, M1-05 · Estimate: 2d · Features: F2.2, F2.4_
