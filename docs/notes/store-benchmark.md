# Store benchmark and backend decision (M1-15, decision #7)

Tool: `tools/store-bench` (`cargo run --release -p store-bench -- --backend docs|loro|mem --photos N`).
Machine: Apple-silicon MacBook Air, release build, loopback only. One backend per process; **both replicas live in the same
process**, so peak RSS covers two replicas (halve it for one). Raw numbers are in `store-bench-results.jsonl`.

What each run does: replica A is filled with N photos (3 blob refs, EXIF incl. GPS, ≈ 350 B encoded) and N original holdings;
an empty replica B syncs from A (**initial sync**); 100 favorites are changed on A and B catches up (**incremental**).
"Disk" is what one replica needs to persist: the docs backend's directory (redb + blob store), or one Loro snapshot.

## Results (originals-only holdings, 100 edits)

| Backend | Photos | Populate | Initial sync | Incremental (100 edits) | Disk / replica | Peak RSS (2 replicas) | Timeline page | `at_risk` (all / first 100) |
|---|---|---|---|---|---|---|---|---|
| mem (baseline) | 100k | 0.1 s | 0.4 s¹ | 0.3 s | – | 990 MB | 21 µs | 16 ms / 5 µs |
| **Loro** | 10k | 0.04 s | 0.06 s | 14 ms | 6 MB | 154 MB | 18 µs | 1.3 ms / 4 µs |
| **Loro** | 100k | 0.4 s | **0.8 s** | 0.23 s | **60 MB** | 1.25 GB | 22 µs | 16 ms / 5 µs |
| **Loro** | 500k | 2.8 s | **6.8 s** | 1.5 s | **303 MB** | 2.75 GB | 23 µs | 221 ms / 5 µs |
| **Loro**, holdings for thumb+preview+original | 100k | 0.5 s | 1.8 s | 0.46 s | 104 MB | 1.93 GB | 20 µs | 32 ms / 6 µs |
| iroh-docs | 10k | 1.4 s | 3.3 s | 83 ms | 51 MB | 237 MB | 17 µs | 1.3 ms / 4 µs |
| iroh-docs | 30k | 4.3 s | 9.8 s | 185 ms | 101 MB | 629 MB | 18 µs | 4.2 ms / 4 µs |
| iroh-docs | 60k | 9.6 s | 28.7 s | 295 ms | 202 MB | 1.3 GB | 18 µs | 9 ms / 5 µs |
| iroh-docs | **100k** | 17 s | **did not complete**² | – | – | – | – | – |

¹ After fixing a quadratic bug in `Effect::touch` found by this very benchmark (it was 18 s).
² Populate and holdings publishing finished; during the initial sync an iroh-blobs worker panicked
(`poisoned storage should not be used`, `iroh-blobs 0.103.1 store/fs/bao_file.rs`) and the sync never finished (reproduced). 100k
photos + 100k holdings = 200k one-entry blobs transferred in a single sync. 500k was not attempted for iroh-docs.

Query latency is the same for every backend by construction (all use `LibraryState`): a 100-photo timeline page costs ~20 µs even
at 500k; `at_risk` over the whole library is 16 ms at 100k and 221 ms at 500k (a page of 100 is 5 µs).

## Reading the numbers

- **Scaling:** iroh-docs initial sync is roughly linear at ~0.3–0.5 ms per record (10k: 3.3 s → 60k: 28.7 s) and falls over at
  100k. Loro stays at ~14 µs per record (500k in 6.8 s). Disk: docs ≈ 3.4 KB/record, Loro ≈ 0.6 KB/record (5–8×).
- **Why docs is slow:** every record is an entry *plus* a separate content blob that must be fetched, and every sync round also
  costs an O(n) digest listing on both sides (our workaround for iroh-docs' unreliable sync result, see assumptions A-047).
  Loro ships values inline in one binary update stream.
- **Memory is a shared problem:** even the in-memory backend needs ~490 MB per replica at 100k (~5 KB/photo), because every photo
  record is held in `LibraryState`. That is a design cost of "answer queries from memory" in *both* backends and the first thing to
  optimise for phones (intern strings, drop unused EXIF, page records from disk). It must be measured on a phone-class device
  (not available here; run `store-bench` on an old iPhone/Android harness or a Raspberry Pi — the numbers above give per-record costs to extrapolate).
- **Holdings granularity matters:** publishing holdings for thumbnails and previews as well as originals tripled the entry count
  (Loro 60 → 104 MB, sync 0.8 → 1.8 s). Every device holds every thumbnail/preview by policy, so only originals need holdings.

## Caveats (what these numbers do *not* say)

- The Loro spike has **no network transport and no persistence layer** (sync is in-process Loro export/import); a real transport
  adds protocol round trips, retries and the verification we needed for iroh-docs. Its sync times are a lower bound.
- The docs numbers include real loopback networking, our digest verification and content-fetch logic, and 100% of its overhead.
- Relay/WAN behaviour, phones, and a second keeper joining a 500k library were not measured.
- Loro has **no access-control model** (see `loro-spike.md`); iroh-docs does.

## Decision

**Loro wins the benchmark on every metric that grows with library size, and iroh-docs as used (one entry + one blob per record)
does not reach the 100k-photo target.** The decision gate therefore triggers. Recommended and recorded:

1. **Personal library → Loro** (new issue **M1-16 "Switch the library backend to Loro"**, to be done before building more on top).
   All devices of a personal library belong to one person and will hold signed device certificates (M2), so the missing
   per-document capability matters less there: authenticity can be enforced by verifying device signatures on import.
2. **Shared albums stay on iroh-docs** (M5). They need real capabilities (read-only members, "can add", invites) which iroh-docs
   provides, and albums are bounded (≤ 50k items, 50 members), which is inside the range iroh-docs handled well (≤ 60k).
3. The `LibraryStore`/`AlbumStore` trait boundary makes this a per-space choice; M2–M4 only see the trait.
4. Independently of the switch: publish holdings for **originals only**, shrink `LibraryState` memory, and keep the iroh-docs
   backend (hardened, with the findings in the assumptions log) as the album backend and the conformance reference.

This is a large decision (re-estimate: **+16 dev-days**, see M1-16) made on a benchmark with the caveats above. It should be
reviewed before M1-16 starts; if you prefer to stay on iroh-docs, the alternative is to batch records into per-month entries
(≈100× fewer entries and blobs) at the price of write amplification and a coarser merge unit, which needs its own spike.
