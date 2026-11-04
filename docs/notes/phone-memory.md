# Phone memory at 100k photos (M4-11 / M6-02 spike)

The budget is under 300 MB on a phone during a 100,000-photo sync. This note measures where a
library replica's memory goes, tries a windowed `LibraryState`, and lists what the real change needs.

**Result: not met, and windowing `LibraryState` alone cannot meet it.** A phone replica that has received
100k photos holds **1.22 GB** (peak **2.1 GB** during the sync). `LibraryState` is about a quarter of
that. Loro's document is most of the rest, and it is dominated by holdings records (six per photo
here). The windowed state cuts `LibraryState` from 335 MB to 25 MB. Getting under 300 MB also needs
holdings stored as a few large records instead of one record per blob, and a sync that folds in
chunks. Both are format or behaviour changes, so none of this is committed as code. Only the
measurement is committed.

## How it was measured

`crates/shoebox-store-loro/examples/phone_memory.rs`, release build, on the Mac (M-series).

```
cargo build --release -p shoebox-store-loro --example phone_memory
B=target/release/examples/phone_memory
$B 100000 fields; $B 100000 state; $B 100000 loro; $B 100000 windowed
$B 100000 loro-photos; $B 100000 loro-per-device; $B 100000 loro-bucketed
$B 100000 build; $B 100000 sync; $B 100000 open     # whole backend; PHONE_MEMORY_DIR sets the work dir
```

* A counting allocator (`cap`) gives live heap bytes for each part and the process peak. Each part
  runs in its own process, so each peak belongs to that part.
* The library is the store benchmark's photo (three JPEG blobs, an iPhone EXIF summary with GPS),
  shuffled dates, and **6 holdings a photo**: a keeper and a Mac each hold the original, preview and
  thumb. A phone that also announces its thumbs and previews would add 2 more. The old estimate in
  `scale-endurance.md` (5–7 KB a photo) counted one holding a photo, which is why it was lower.
* Loro-only measurements store the real keys and values, with signed values (`0x01 ++ 64-byte
  signature ++ postcard`). The whole-backend runs (`build`/`sync`/`open`) use the in-process backend,
  which has no keys and so stores values 64 bytes shorter.
* **The S25 was not connected** (`adb devices` listed only an emulator), so there are no phone
  numbers. The heap counts are the same on any 64-bit target. Phone RSS adds allocator (Scudo)
  overhead on top of them.

## Before: where the memory goes (100k photos)

### Whole replica (`LoroBackend`, persisted, unsigned values)

| Situation | Live | Per photo | Peak |
|---|---|---|---|
| Empty replica after receiving the whole library | **1,220 MB** | 12.2 KB | **2,135 MB** |
| ... while also holding `photos_by_time(Page::all())` | 1,292 MB | 12.9 KB | |
| Replica opened from disk (app start) | 1,112 MB | 11.1 KB | 1,855 MB |

### The parts

| Part | MB | B/photo | Notes |
|---|---|---|---|
| `LibraryState`: photo records + timeline | 91 | 906 | `BTreeMap<PhotoId, PhotoRecord>` + `BTreeSet<(Reverse<Timestamp>, PhotoId)>` |
| `LibraryState`: 6 holdings | 244 | 2,438 | 406 B per holding: `holdings` keyed by (32 B device, 32 B blob) with an `Lww` (48 B HLC), plus `holders_by_blob` |
| **`LibraryState` total** | **335** | **3,345** | |
| Loro document, loaded from a snapshot and read (what `fold_all` does) | **732** | 7,324 | 136 MB until the first read. Reading decodes the map state, and the encoded snapshot stays in memory |
| Loro document, received as updates (a sync) | 436 | 4,364 | history plus state, about 1.75× the raw keys and values |
| Other (backend run minus the two above) | ~150 | ~1,500 | not broken down: caches left by the import's `diff`, the update log, `Effect`s |
| Keys and values themselves | 248 | 2,481 | photo record: 88 B key + 431 B value. Holding: 155 B key + 172 B value |

`PhotoRecord` itself is 480 B inline + 93 B of strings = 573 B, and a `Vec` of all 100k is 57 MB.
Field by field (inline):

| Field | Bytes |
|---|---|
| `id` | 32 |
| `original`, `preview`, `thumb` (`BlobRef`: hash 32 + size 8 + mime `String` 24) | 3 × 64, + 3 × 10 heap |
| `taken_at` | 8 |
| `exif` (make/model/lens `Option<String>`, width, height, gps) | 96, + 63 heap |
| `imported_by` | 32 |
| `favorite: Lww<bool>` (an HLC is 48 B: it carries a 32 B device id) | 56 |
| `deleted_at: Lww<Option<Timestamp>>` | 64 |

Most of the record is hashes and HLCs, which no screen needs until a photo is opened.

**Why the peak is 2.1 GB:** a sync that brings the whole library folds every changed key in one batch.
`fold_keys` first copies every value out of Loro (`Vec<(String, Vec<u8>)>`, about 250 MB), then decodes all
of them into `StateEntry`s before applying any (`Vec<Judged>`). The snapshot or update bytes are held
during all of this.

## Holdings dominate Loro too

Loro document per layout. The same 100k photos, signed values:

| Layout | Records | Raw keys + values | Received as updates | From a snapshot, read |
|---|---|---|---|---|
| Today: photo + 6 holding records (one per device and blob) | 700k | 248 MB | 436 MB | 732 MB |
| Photo records only (the floor for one record a photo) | 100k | 52 MB | 79 MB | 169 MB |
| Photo + 1 holdings record per (device, photo), kinds as bits | 300k | 85 MB | 166 MB | 311 MB |
| Photo + holdings in buckets of 256 photos per device | 101k | 59 MB | 85–92 MB | 204 MB |

Loro costs about 600 B (updates) to 940 B (snapshot) per holding record. Counting `LibraryState` too,
**one holding costs about 1.0–1.35 KB per photo**. Loading from a snapshot costs about twice as much as
receiving the same document as updates, in every layout. Load times on the Mac: today's layout 1.0 s
(snapshot) and 1.5 s (updates), bucketed 0.2 s for both.

## The prototype: windowed state

The prototype is `windowed::Columns` in the example. `LibraryState` keeps only what screens and
queries use for every photo:

| Column | Bytes a photo |
|---|---|
| `index: HashMap<PhotoId, u32>` | ~50 |
| `ids: Vec<PhotoId>` | 32 |
| `taken_at: Vec<i64>` | 8 |
| `flags: Vec<u8>` (favorite, deleted; room for motion, keeper-backed) | 1 |
| `thumb: Vec<Blake3Hash>` | 32 |
| `timeline: BTreeSet<(Reverse<i64>, u32)>` (live photos, newest first) | ~25 |
| `held: Vec<Vec<u8>>`: kinds held, as bits, per device and photo | 1 per device |
| **Measured total** | **148 B (15 MB)** |
| optional: preview/thumb hash → photo, for `holders(blob)` on non-original blobs | +96 B (10 MB) |

Full records are decoded from the Loro map on demand. Loro already holds every record's bytes, so
the full `PhotoRecord` map in `LibraryState` is a second copy. Measured on the Mac: a 200-photo
timeline page, ids from the columns and records decoded from Loro, takes **1.3 ms**. Decoding all
100k records once takes 250 ms.

### After (100k photos, 6 holdings a photo)

These totals are projections: they add up parts that were each measured. The whole backend was not run
with these changes.

| Variant | `LibraryState` | Loro | Other | Total |
|---|---|---|---|---|
| Today | 335 | 732 (snapshot) / 436 (updates) | ~150 | **1,220** (peak 2,135) |
| Windowed state only | 25 | 732 / 436 | ~150 | **~600–900** |
| Windowed + holdings in per-device buckets | 25 | 204 / 92 | ~20 (scales with records) | **~140–250** |
| Same, holdings as 1 record per (device, photo) | 25 | 311 / 166 | ~40 | ~230–380 |

The last two rows also need the sync to fold in chunks (issue 3). Without that, the peak stays near
twice the live size.

**Not landable as a contained change.** The windowed state alone leaves 600–900 MB. The 300 MB budget
needs the holdings format to change, which touches the replicated data (a new record kind, migration,
rotation) and every backend. The prototype code is therefore not committed as a change to
`LibraryState`. It stays in the example as the design reference.

## Design

1. **Columns in `LibraryState`, records in the store.** Replace `photos: BTreeMap<PhotoId, PhotoRecord>`
   with the columns above. Add a `RecordSource` that `LibraryState` reads records through:
   `fn photo(&self, id) -> Option<PhotoRecord>`. In the Loro backend it decodes from the Loro map. In the
   mem backend it is a map. In the docs backend it is the iroh-docs entry. `merge_photo` reads the
   existing record through the source (a decode of about 2.5 µs) and writes the columns from the
   merged result.
2. **Deleted photos:** a sparse map `PhotoId → deleted_at` for Recently Deleted and `purge_expired`.
   Only deleted photos have an entry.
3. **Holdings as bits per (device, photo index)** for the three kinds of a photo's blobs, plus the
   generic map for other blobs (index shards, motion videos, of which there are few). `copy_count`,
   `held_by_keeper` and `at_risk` read the bits. A device's role is looked up once per device, not per
   photo.
4. **Holdings replicated in buckets**: one single-writer record per (device, bucket of photos), for
   example keyed by the first byte of the photo id (256 buckets). The writer is the only author, so no
   HLC per holding is needed in memory. This is the change that brings Loro from about 730 to about
   200 MB.
5. **API unchanged for the timeline:** `photos_by_time(range, page)` walks `timeline` and decodes
   `page.limit` records. Add id- and column-level reads (`photo_ids_by_time`, `photo_facts(id) ->
   (taken_at, flags, thumb)`) for callers that today ask for every record.

## Issues for the real change

1. **Holdings format** (largest gain): add bucketed per-device holdings records to `entry_codec`. Read
   both forms during a transition, and have each device republish its own holdings in the new form.
   Also update the role rules in `judge`, `Carry` in rotations, and the docs backend. Pick the bucket
   size against Loro's history growth: every change rewrites the whole bucket value, and history keeps
   every version until compaction. Measure a week of changes, and consider shallow snapshots for the
   history.
2. **Windowed `LibraryState`** with a `RecordSource` (design points 1–3 and 5). Implement it for all three
   backends, and run the conformance suite against each.
3. **Fold a sync in chunks**: `Inner::fold_keys` copies and decodes every changed record before
   applying any, so the peak is 2.1 GB against 1.2 GB live. Fold about 5,000 keys at a time. This is
   contained and worth doing first.
4. **Loading from a snapshot doubles Loro's memory** (732 against 436 MB, and 169 against 79 MB for photos
   only): the encoded state seems to stay after the map state is decoded. Find out whether Loro can drop
   it. Otherwise persist differently (updates, or a shallow snapshot plus history kept on disk), and
   measure open time on the S25.
5. **Callers of `photos_by_time(TimeRange::ALL, Page::all())`**: 16 call sites in the app (`core.rs`),
   the keeper (`status.rs`) and `shoebox-node` (`compact`, `maintenance`, `search_status`, `holdings`,
   `freeup` ×2, `indexer` ×3, `search`, `web_share/cloud_keeper`). Each one makes 57 MB of records
   transiently, and decoding them costs 250 ms with the windowed state. Move them to the column reads.
6. **Measure on the S25**, as RSS (`dumpsys meminfo`) as well as heap. Use a signed library: the
   whole-backend runs here are unsigned, and the signature-verdict cache was not measured at 100k.
7. **Break down "other"** (~1.5 KB a photo in the backend run): the caches left by Loro's `diff` on import,
   the update log, and anything else.
8. **Strings in `PhotoRecord`**: the mimes and EXIF names could be interned (93 B a photo). This matters
   only for records that are decoded, so it is low priority once records are windowed.
