# Library memory at 100k photos (M4-11 spike, 2026-11-05)

The budget is under 300 MB of app memory on a phone during a 100k-photo sync. The earlier estimate was 5–7 KB per photo
per replica (500–700 MB), from the RSS of two replicas in one process (`scale-endurance.md`, `store-benchmark.md`). This note
measures where that memory goes, changes what was cheap to change, and lists the rest.

## How it was measured

`tools/store-bench` now counts live heap bytes with a counting global allocator. RSS mixes both replicas, freed pages the
allocator keeps, and the allocator's slack. The probe measures at four points:

* replica A after its photos;
* replica A after its original holdings;
* replica B once it has synced from A;
* replica B opened again from its own files, as a phone does on every start.

It also records the most the sync held at once above what A already had. That last number includes A's side of the exchange,
because both replicas run in one process.

`--bin loro-memory` takes Loro alone apart: a document of photo-sized records, exported and imported the way a replica
receives a library.

All runs: Apple-silicon MacBook Air, release build, 100,000 photos, holdings for originals only, real iroh nodes on loopback,
signed records (`--backend loro-net`). Raw lines are in `store-bench-results.jsonl`.

## Where it went (before)

| Replica B, per photo | Live heap |
|---|---|
| Synced from A | **5.9 KB** (593 MB) |
| Opened again from its files | **4.9 KB** (492 MB) |
| Most held during the sync, above A | 977 MB |

* **The Loro document, about 3.5 KB a photo.** Compaction saved a full Loro *snapshot*. A document opened from a snapshot
  costs 3.7 KB a photo once its map is read; the same document imported from its *updates* costs 1.1 KB.
  * Exporting a snapshot also leaves the exporting document at 4.7 KB a photo. Loro keeps encoded copies of the blocks it
    wrote.
  * Re-exporting each imported update to the log did the same thing on a smaller scale: +0.6 KB.
  * The `diff` after every import builds a checkout cache as large as the document: 190 MB transient at 100k, 640 MB when
    the document came from a snapshot.
* **`LibraryState`, 1.5 KB a photo** (the `mem` backend):
  * 0.9 KB for the photo record. Six heap strings and three 32-byte device ids, nearly all the same values across a library.
  * 0.6 KB for its holding. Each blob's holders sat in a one-element `BTreeSet`, which allocates a whole B-tree node.
* Signature verdicts and the node itself: small (under 0.2 KB, 1.4 MB).

## What changed (landed with this note)

1. **Replicas compact to Loro's update form**, not a snapshot. Older replicas still open, since Loro imports either form; a
   test opens one an earlier build saved.
2. **An import is logged as the bytes that arrived**, not re-exported, when nothing local was waiting to be logged.
3. **Loro's diff and history caches are freed after each import.** A first sync into an empty replica takes every key
   without a diff.
4. **`LibraryState` keeps photos packed** (`photo_table.rs`):
   * MIME types, camera names and device ids are stored once and referred to by index;
   * records go in and out as plain `PhotoRecord`s.
5. **A blob's holders are a sorted list.**

| Replica B, per photo | Before | After |
|---|---|---|
| Synced from A | 5.9 KB (593 MB) | **2.0 KB (203 MB)** |
| Opened again from its files | 4.9 KB (492 MB) | **2.5 KB (252 MB)** |
| Most held during the sync, above A | 977 MB | 546 MB |
| `LibraryState` alone (`mem`, photo + holding) | 1.54 KB | 0.86 KB |
| Populating A (100k writes) | 30 s | 2.7 s |
| Initial sync / reopen | 4.2 s / 1.7 s | 3.5 s / 1.7 s |
| Incremental sync, 100 edits | 8 ms | 36 ms |
| Timeline page / `at_risk` page | 21 µs / – | 21 µs / – |

The incremental sync is slower because the freed diff cache is rebuilt on the next import. At 100k that costs about 30 ms a
round, which is cheaper than keeping roughly 26 MB.

Populating got ten times faster as a side effect: each compaction during a burst of writes used to export a full snapshot.

## Is the budget met?

* **The library at rest: yes on the Mac's allocator numbers.** A reopened replica is 252 MB at 100k, a synced one 203 MB.
* **Not shown: the phone itself.** RSS on the S25 is higher than live heap (allocator slack, the WebView, the runtime), and the
  search data adds about 52 MB idle (`scale-endurance.md`).
* **During a first sync of 100k photos: no.** The whole library travels as one message.
  * The receiver holds the message body, a decoded copy, Loro's parse of it, and the state it is building, all at once.
  * That is several hundred MB of transient memory on top of the 200 MB the replica ends up with.
  * The figure above, 546 MB, also counts A's side of the exchange.

## Still to do (issues)

* **M4-11a · Sync in bounded batches.**
  * The server answers `Hello` with at most N changes (Loro `ExportMode::updates_in_range`) and a "more" flag. The client
    keeps asking. Each batch is imported, logged and folded before the next.
  * The peak becomes the batch size, not the library.
  * This is a protocol change: `shoebox/loro/1` beside `/0`, and both stay served for a release.
* **M4-11b · Decode the message once.**
  * `wire::read_msg` reads the body, then postcard copies the update bytes out of it.
  * Borrowing (`serde_bytes`, `from_bytes` into `&[u8]`) halves the transient on the receiving side even before batching.
* **M4-11c · Measure on the S25.** Run `store-bench --backend loro-net --photos 100000` as an Android test binary, or open the
  app on a 100k library made by `shoebox-dev`, and read `dumpsys meminfo` at rest and during the first sync.
* **M4-11d · A shallow document on phones (design first).**
  * A Loro shallow snapshot keeps the state and drops the history, which would leave the document at roughly the size of its
    values.
  * The cost: a phone could no longer bring a peer up to date from before its shallow point. That peer would need another
    device, or a full snapshot.
  * That conflicts with "features never need another device online" (only one-time processing may be split), so it needs a
    design that keeps any two devices able to sync.
* **Signature verdicts are saved before the records of a big import are folded,** so the first save after a first sync is
  empty. They are written again at the next idle compaction, but a phone closed before that re-checks every signature on its
  next start (1.7 s at 100k here).
