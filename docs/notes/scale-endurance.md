# Scale and endurance (M6-02)

What has been measured, what has not, and what failed the acceptance criteria.

## Measured (laptop, loopback; `tools/store-bench`, see `store-benchmark.md`)

| Question | Result |
|---|---|
| Library of 100,000 photos syncs to an empty device | 0.8 s (Loro); 500,000 in 6.8 s |
| Disk for the library metadata | 60 MB at 100k, 303 MB at 500k per replica |
| Timeline page / at-risk list at 100k | 22 µs / 16 ms |
| Shared album of 50,000 photos | written in about a minute; a new member has all 50,000 items 50 s after joining (see below) |
| Interrupted download resumes | only the missing ranges are fetched again (`fetcher.rs: a_partly_downloaded_blob_resumes…`) |
| Free-up, revocation, late keeper, disk full, clock skew | `failure-scenarios.md` |

## Search at scale (S1-20, Mac, release builds)

| Measure | 100k photos | Budget |
|---|---|---|
| Facts query (`kerala 2019 pixel 8`) | under 50 ms | 50 ms |
| Meaning: scoring every photo's vector (512-d, 8-bit, streamed from `search/vec-*.bin`) | 8 ms | 50 ms (Mac), 150 ms (Pi 5, S25) |
| Re-labelling from vectors (937 labels) | 22 s | 2 min |
| Text query encoding | 14 ms | 50 ms (phone) |
| `facts.idx` | under 15 MB | — |
| Vectors on disk (`vec8` + `vec1` + ids) | about 59 MB | — |
| Face shards (one face a photo, 161 bytes each) | about 16 MB | 50 MB |
| Gazetteer lookups (100k) | 0.3 s | — |

Not measured: the S25, a Pi 5, nest; RSS while searching on a phone (the text model alone takes about 510 MB
resident on the Mac while loaded, so a phone loads it on the first search by meaning and lets it go after two
minutes); the week-long endurance run.

## Not met yet

* **Mobile memory under 300 MB during a 100k-photo sync: not met.** The library state costs about
  5–7 KB per photo per replica in memory (100,000 photos ≈ 500–700 MB), because every photo record is
  held in full. A phone with 4 GB survives it, a 2 GB phone may not. The fix is a windowed/lazy state
  (keep ids, dates and flags in memory, load records on demand); it is a design change in
  `LibraryState`, ticketed here rather than patched.
* **A 600 GB keeper sync with interruption and resume:** not run (no such disk or corpus was
  available). The per-blob resume is tested at small scale; the large-scale claim "at most one
  restart-induced re-download of any chunk" is inferred from iroh-blobs' range resume, not measured.
* **Raspberry Pi 4 profile:** not run (no Pi). The release workflow builds the aarch64 keeper
  (`release.yml`, never executed).
* **ETA accuracy:** the estimate is the recent rate of the at-risk count (and of previews arriving). It
  has been exercised on small libraries only.

## Findings while testing at scale (fixed)

* iroh-docs stalls its whole engine if an event subscriber stops reading. A 50,000-entry album join
  scanned before reading events and deadlocked; events are now drained into an unbounded queue
  from the moment of subscribing.
* Blob GC walks every docs record each pass. With the testkit's 200 ms GC interval that starves a big
  sync; production uses five minutes and only runs when the node has been idle for 30 seconds.
* Joining an album no longer waits for every photo: it needs the album's own record and then streams the
  rest, so the join takes seconds whatever the album size.

## Album of 50,000 photos to a new member (M5-01 acceptance)

`cargo test -p shoebox-store --test shared_scale -- --ignored --nocapture` (about two minutes).

Measured on the development laptop (loopback, release-like GC settings): the owner writes 50,000 items in
about 55 s; a new member joins and holds all 50,000 items **50 s** after presenting the invitation
(limit: 2 minutes). Most of that is waiting for the album's own record to arrive in iroh-docs' sync order.
