---
id: M6-05
title: On-disk format versioning and migrations
milestone: M6 · Hardening & beta
labels: [area:core, area:node, type:feature]
estimate_days: 2
depends_on: [M5-08]
features: [F8.5]
---

# M6-05 · On-disk format versioning and migrations

## Why

Beta users' data has to survive app updates.

## Scope

- Schema/version markers for the store, config and blob tags; a migration runner with backup-before-migrate.
- Mixed-version sync compatibility rule (N and N-1 must interoperate) with a test.

## Acceptance criteria

- [ ] Upgrading from a previous beta build migrates a fixture library without loss (CI test).

---
_Depends on: M5-08 · Estimate: 2d · Features: F8.5_
