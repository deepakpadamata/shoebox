# Progress

Implementation status per issue. Assumptions made along the way are logged in
[`docs/notes/assumptions.md`](../docs/notes/assumptions.md) for review.

## M0 · Foundations — done

| Issue | Status | Notes |
|---|---|---|
| M0-01 Pin iroh crate versions | ✅ | `docs/notes/iroh-versions.md`; `examples/hello-sync` runs (loopback only) |
| M0-02 Workspace scaffold & licensing | ✅ | Apache-2.0/MIT libraries, AGPL-3.0 apps |
| M0-03 CI with license guard | ✅ | Workflow written; guards verified locally (no remote yet, so CI itself has not run) |
| M0-04 Core domain model types | ✅ | Golden-bytes tests lock the v1 encoding |
| M0-05 Merge rules | ✅ | proptests for commutativity/associativity/idempotence |
| M0-06 Store traits & in-memory backend | ✅ | |
| M0-07 Store conformance suite | ✅ | 12 scenarios; `shoebox-store-mem` passes |
| M0-08 Fixture photo corpus | ✅ | `cargo xtask fixtures`; RAW sample is a synthetic stand-in (see A-023) |

## Next: M1 · Sync core
