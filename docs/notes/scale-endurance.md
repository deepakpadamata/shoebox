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

### On the S25 and nest (2026-10-30)

`device_bench` (`crates/shoebox-ml/examples/device_bench.rs`) at 100k synthetic photos, one thread, release builds;
full table in `search-models.md`:

| Measure | Mac | nest (Ryzen 5 3600, musl) | S25 | Budget |
|---|---|---|---|---|
| Facts query, p50 / p95 | 2.1 / 2.5 ms | 5.4 / 6.5 ms | 2.6 / 3.0 ms | 50 ms |
| Meaning: scoring 100k vectors, p50 / p95 | 7.5 / 7.9 ms | 26 / 28 ms | 17 / 18 ms | 150 ms (S25) |
| Text query encoding, p50 / p95 | 14 / 15 ms | 95 / 102 ms | 68 / 69 ms | 50 ms (phone): **not met** |
| A search by meaning, end to end (encode + score + facts) | about 25 ms | about 135 ms | about 90 ms | p95 150 ms: met |
| Text model resident while loaded | 590 MB | 200 MB | 174 MB | — |
| Image encoding (indexing), ms/photo | 35 | 296 | 285–465 | — |

The real library (5,766 photos), on the devices:

* **S25** (debug APK, text model only): "waterfall" finds 53 photos, all of waterfalls, from vectors the Mac and nest
  made. Search data on the phone: `facts.idx` 1.3 MB, `vec-*.bin` 3.5 MB (written on the first search by meaning).
  App memory (`dumpsys meminfo`, native heap): 270–285 MB idle, 461–467 MB after a search by meaning, back to 284 MB
  2.5 minutes after leaving the search screen. While the search screen stays open the text model stays loaded:
  each refresh of the results (sync brings changes) counts as a use, even when the query's vector is cached.
* **nest** (keeper, musl, models in `/usr/local/share/shoebox/models`): indexes at about 340 photos a minute with
  half its cores (the Mac: about 1,800); keeper RSS 0.86–1.0 GB while indexing with all models loaded. Search data
  4.7 MB under `search/`, 10.3 MB of shard blobs.

**The S25 alone (2026-10-30):** airplane mode, Wi-Fi, mobile data and Bluetooth off, the app restarted with no
network. Every kind of search answered, from the data already on the phone:

| Query | Understood as | Result |
|---|---|---|
| `waterfall` | words, by meaning | 53 photos, all waterfalls (the same 53 as online) |
| `october 2026` | a date | 96 photos |
| `india` | a place | 1,835 photos |
| `samsung sm-s931b` | a camera | 2 photos (correct: the library's other S25 photos are named "samsung Galaxy S25") |
| `gagana` | words | the photo of a sign reading GAGANA first, then chat screenshots with the name (text in photos) |
| `mom waterfall` | a person and words | 16 photos of Mom at waterfalls |

People (Albums → People) showed the named groups and their face crops. The app said "Syncing…" while it had no
network to sync over. Not done: the Mac and nest alone (the Mac holds every shard and model, nest is the keeper).

**What search holds while idle**, at 100k synthetic photos (`device_bench --idle`, a fresh process loading the
index files as the app does at start), against a budget of 30 MB: **not met.**

| | facts index | meaning index | total |
|---|---|---|---|
| S25 | 38 MB | 14 MB | **52 MB** |
| Mac | 59 MB | 8 MB | **67 MB** |

The 512-d int8 vectors stay on disk and are read for each search; the meaning index holds ids and sign bits. The
facts index keeps every photo's facts as strings and the word postings of every field. At the real library's
5,766 photos that scales to about 3–4 MB. No text or image model is loaded while idle.

Not measured: a Pi 5 (none at hand); the Mac and nest with every other device off; the week-long endurance run.

### For S2 (seen while measuring, not fixed in S1)

* **Idle memory at 100k photos: 52 MB on the S25, 67 MB on the Mac (budget 30 MB), most of it the facts index.** Keep facts in compact columns (dates, flags, ids of
  interned cameras, lenses and places) and the word postings as sorted integer arrays, or map `facts.idx` instead
  of decoding it.
* **Phone text encoding: 68 ms against 50 ms.** A smaller or more quantised text model, or tract's newer kernels.
* **The text model (174 MB on the S25) stays loaded while the search screen is open:** refreshing the results
  counts as a use even when the query's vector is cached. Count only encodings as use.
* **Camera names are understood only whole:** "samsung", "galaxy" and "galaxy s25" are searched as words, not as
  the camera "samsung Galaxy S25". Match a camera on its model or make alone, and treat "SAMSUNG"/"samsung" variants
  as one name in suggestions.
* **"Syncing…" with no network:** say "Offline" instead.

## Not met yet

* **Mobile memory under 300 MB during a 100k-photo sync: not met.** Measured in `phone-memory.md`: a
  replica that has received 100k photos (6 holdings each) holds 1.22 GB, with a peak of 2.1 GB. About
  a quarter of that is `LibraryState` and most of the rest is Loro's document, which is dominated by
  the one-record-per-blob holdings. A windowed state is not enough on its own. The budget also needs
  holdings replicated in buckets and a sync that folds in chunks.
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
