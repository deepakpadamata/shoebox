---
id: M1-13
title: Delete, Recently Deleted and blob GC
milestone: M1 · Sync core
labels: [area:node, area:core, type:feature]
estimate_days: 1.5
depends_on: [M1-11]
features: [F2.8]
---

# M1-13 · Delete, Recently Deleted and blob GC

## Why

A delete must reach every device, stay recoverable for 30 days, and then actually free space.

## Scope

- `mark_deleted`, `restore`, `purge_expired(now)` (30 days).
- After a purge, remove blob tags and remove holdings; GC runs on every node.
- Shared-album references (M5) block GC of the blobs they use; leave a hook for that now.

## Acceptance criteria

- [ ] Delete → restore round-trips on all nodes.
- [ ] After purge plus GC, no node still stores the original (testkit with fake clock).

---
_Depends on: M1-11 · Estimate: 1.5d · Features: F2.8_
