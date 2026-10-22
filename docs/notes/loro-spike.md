# Loro backend spike (M1-14)

**Question (decision #7):** should the replicated library metadata live in iroh-docs or in a Loro CRDT document?
This spike builds just enough of a Loro backend to compare them fairly. Measurements are in
[`store-benchmark.md`](store-benchmark.md) (M1-15); this note is about what exists and what it would take to finish.

## What was built

`crates/shoebox-store-loro` implements `LibraryStore` and `StoreBackend` on a `loro::LoroDoc`:

- The merge rules, indexes and queries are the shared `core::library_state::LibraryState`, exactly as in the mem and docs backends.
- Every replicated record is a binary value in one Loro map, keyed `<author>/<record key>` with the same key layout and
  value encoding as the iroh-docs backend (`core::entry_codec`). Each device writes only under its own author prefix, so
  Loro never has to resolve two concurrent writes to one key; cross-device conflicts are resolved by `core::merge` on
  read (the same design as iroh-docs, where entries are per-author too).
- Replicas converge by exchanging Loro updates (`export(updates since the peer's version vector)` / `import`).
- `export_snapshot()` gives the whole document as one blob (what would be stored in iroh-blobs).
- It passes **12 of the 13 conformance scenarios** (everything except shared albums, which are out of scope for the spike).

## What it is not

| Gap | Why it matters | Effort to close (solo, focused days) |
|---|---|---|
| **No transport.** Sync is simulated in-process (real Loro export/import, no network). | Needs an iroh protocol (or gossip announcements + snapshot/updates as blobs) with version-vector exchange, retries, backoff and the same "did it really converge?" verification we needed for iroh-docs. | 3–4 |
| **No persistence** beyond `export_snapshot()`. | Snapshot + incremental updates as blobs, compaction (`ShallowSnapshot`), crash safety. | 2–3 |
| **No access control model.** | iroh-docs gives a real read-only capability (a namespace public key). A Loro document has none: anyone holding it can write. Read-only viewers, "can add" album members and revocation would need signed updates verified on import (M2's device certs make this possible) and a *separate* answer for shared albums. This is the largest gap and a security-relevant one. | 5–8 |
| **Shared albums not implemented.** | Separate documents with membership, invites and roles on top of the missing access control. | 4–6 |
| **Rescan is O(n).** | The spike re-reads the whole map after each import; Loro's event subscriptions would give the changed keys directly. (iroh-docs has the same O(n) rescan today.) | 1 |
| **History grows.** | Loro keeps full history; an overwritten record still costs bytes until compaction. Photos' favorite/delete edits and holdings churn make this real. | part of compaction above |
| **Whole document in RAM.** | Loro loads the doc into memory; fine on a desktop, to be measured on a phone-class device. | measure (M1-15) |

**Total to reach parity with the docs backend: roughly 3–4 weeks**, dominated by access control and shared albums.

## Where Loro would win

- Record *values* are inline in the document, not one blob per entry as in iroh-docs, so syncing a record does not need a
  second round trip to fetch its content. (This is why iroh-docs needed content-fetch retry logic and why 10k small records
  cost 10k blob fetches.)
- A snapshot is a single compact blob: initial sync of a large library is one transfer, not per-entry.
- Richer data types (text, lists, movable lists) if the product ever wants collaborative fields.

## Where it loses

- No built-in capability model (above); iroh-docs's is what shared albums and read-only viewers are built on.
- A younger, faster-moving API surface to track, versus one n0 maintains alongside iroh itself.
- Everything we already hardened on iroh-docs (digest-verified sync, GC protection, content fetch retries) is replaced by new
  work, not reused.
