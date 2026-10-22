---
id: M6-07
title: Diagnostics bundle
milestone: M6 · Hardening & beta
labels: [area:node, area:ui, type:feature]
estimate_days: 1.5
depends_on: [M6-04]
features: [F8.4]
---

# M6-07 · Diagnostics bundle

## Why

When P2P fails in the wild, we need data without collecting personal content.

## Scope

- One-tap export: versions, device list (ids, roles), path stats, recent errors, log tail with redaction (no file names, GPS or names).
- Shared only through the OS share sheet (nothing is uploaded automatically).

## Acceptance criteria

- [ ] Redaction tests: fixture logs containing names, paths and GPS come out clean.

---
_Depends on: M6-04 · Estimate: 1.5d · Features: F8.4_
