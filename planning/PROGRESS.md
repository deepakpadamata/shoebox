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

## M1 · Sync core — done

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
| M1-16 Switch library backend to Loro | ✅ | production Loro backend + `HybridBackend` (Loro library, docs albums); acceptance numbers in `docs/notes/store-benchmark.md`; memory work for phones not done (A-066) |

## M2 · Identity, pairing & keeper — done except M2-03b (split off) and unverifiable acceptance checks

| Issue | Status | Notes |
|---|---|---|
| M2-01 Identity | ✅ | root → admin → device certs, verified in all backends (A-056/057) |
| M2-02 Recovery kit | ✅ | 24-word phrase + printable text; `shoebox/recover/0` handshake lets a device with nothing rejoin through a surviving device (A-068) |
| M2-03 Enforce device list and revocation | ✅ | connection gating + `Removed` state (A-061); rotation is M2-03b |
| M2-03b Rotate capability on revocation | ✅ | epoch rotation on the Loro store, signed notice, offline members catch up (A-067); 100k records in 1.2 s (release) |
| M2-04 Pairing protocol | ✅ | tested on loopback; relay path **not verified** (A-062) |
| M2-05 Keeper daemon & control socket | ✅ | lock file, 0600 socket, resumes after restart |
| M2-06 Keeper CLI | ✅ | `docs/keeper-cli.md` (mockups 5g/5i are not in the repo, A-063) |
| M2-07 Service install | ✅ | unit/plist generation tested; real Raspberry Pi reboot test **not verified** |
| M2-08 Safe free-up-space | ✅ | has-probe ALPN, 7-day confirmation, literal "≥ target other copies" (A-064) |
| M2-09 Release builds | ◐ | workflow + verifying install script written; install script control flow tested with a fake minisign; **no tag was built, no real signature checked** (A-065) |

## Next: M3 · Desktop app
