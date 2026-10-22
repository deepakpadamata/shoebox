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

## M1 · Sync core — done except M1-16 (decision pending review)

| Issue | Status | Notes |
|---|---|---|
| M1-01 Node runtime skeleton | ✅ | file-based key, peer events, partitions for tests |
| M1-02 Testkit multi-node scenarios | ✅ | `Cluster`, partitions, fake clock; 3-node scenarios on the docs backend |
| M1-03 Blob store integration | ✅ | protection tags, reference import (2 GB opt-in test), verified fetch |
| M1-04 Media import pipeline | ✅ | HEIC feature untested (no pkg-config here) |
| M1-05 iroh-docs LibraryStore backend | ✅ | passes the full conformance suite; sync hardened (see assumptions A-036, A-047, A-051/052) |
| M1-06 Folder import with dedupe | ✅ | watched folders persist and catch up |
| M1-07 Developer CLI | ✅ | `docs/notes/demo-m1.sh` runs end to end in a test |
| M1-08 Library sync between nodes | ✅ | relay / different networks **not verified** here (loopback only) |
| M1-09 LAN discovery, relay config, connection reporting | ✅ | mDNS verified on one machine; unplugged/hole-punch checks **not verified** |
| M1-10 Gossip presence | ✅ | |
| M1-11 Holdings and copy counts | ✅ | replicated redundancy target; at_risk(100k) ≈ 22 ms release |
| M1-12 Role-based blob fetcher | ✅ | |
| M1-13 Delete, Recently Deleted and blob GC | ✅ | GC hardened (activity-gated, temp-tag holds) |
| M1-14 Loro backend spike | ✅ | `docs/notes/loro-spike.md` |
| M1-15 Store benchmark and decision | ✅ | Loro wins; iroh-docs fails at 100k. Decision to review: `docs/notes/store-benchmark.md` |
| M1-16 Switch library backend to Loro | ⬜ | new, 16 d; pending review of the decision |

## Next: M2 · Identity, pairing & keeper
