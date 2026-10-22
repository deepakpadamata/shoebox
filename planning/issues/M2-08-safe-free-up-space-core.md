---
id: M2-08
title: Safe free-up-space core
milestone: M2 · Identity, pairing & keeper
labels: [area:node, area:core, type:feature]
estimate_days: 2
depends_on: [M1-11]
features: [F7.3]
---

# M2-08 · Safe free-up-space core

## Why

'Free up space' must never delete the last copies (mockup 6a).

## Scope

- `has` probe over the blobs protocol (or a small custom ALPN) to confirm a peer really holds a hash.
- `safe_to_free(device)`: originals with ≥ target copies elsewhere and at least one holder confirmed within 7 days; never pinned items.
- `free_up_space(dry_run)` returns bytes and a list; the executing run removes tags and updates holdings.

## Acceptance criteria

- [ ] Testkit: with the only other holder offline and unconfirmed for 8 days, nothing is freed.
- [ ] Bytes reported in dry-run equal the bytes actually freed.

---
_Depends on: M1-11 · Estimate: 2d · Features: F7.3_
