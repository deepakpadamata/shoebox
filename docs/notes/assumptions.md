# Assumptions log

Reasonable assumptions made while implementing the issues in `planning/issues/`, recorded for review.
Each entry: **ID** · issue · assumption · why · how to reverse.

## M0

- **A-001 · M0-01** · `shoebox-core` and the other crates use a `0.0.1` workspace version and `publish = false`.
  Why: nothing is released yet. Reverse: bump in `[workspace.package]`.
- **A-002 · M0-02** · `repository` is an `example.invalid` placeholder since no remote exists yet.
  Reverse: set the real URL in `[workspace.package]` once a remote is created.
- **A-003 · M0-02** · Rust edition 2021 and `rust-version = "1.85"` (conservative MSRV); the toolchain itself is `stable`.
  Why: iroh 1.x needs a recent compiler; 2021 avoids edition-2024 churn in macro-heavy deps. Reverse: bump edition/MSRV.
- **A-004 · M0-02** · License files: `LICENSE-APACHE`/`LICENSE-MIT` at the root (copyright "Shoebox contributors",
  text copied from the iroh-docs crate's standard licenses), and `apps/LICENSE-AGPL` (official GNU text).
  `tools/*`, `xtask` and `examples/*` are treated as library-licensed (Apache-2.0 OR MIT), since decision #17
  only names `apps/*` as AGPL. Reverse: change the `license` field in the affected crates.
- **A-005 · M0-02** · `apps/desktop-mobile` is a placeholder `fn main() {}` crate now; the Tauri project replaces it in M3-01.

- **A-006 · M0-03** · CI cannot run yet (no remote), so the workflow in `.github/workflows/ci.yml` is unexecuted;
  its steps were reproduced locally (`cargo fmt/clippy/test`, `cargo deny check licenses sources`,
  `scripts/check-no-app-deps.py`). The "deliberate AGPL dependency" acceptance check was verified locally
  with a registry AGPL crate (`lemmy_utils`) in `shoebox-core` → `cargo deny` failed; reverted.
  Note: cargo-deny skips *path* crates as "private", so only registry deps are covered by the allow-list; edges
  from libraries to `apps/*` are covered by `scripts/check-no-app-deps.py` (verified by giving `keeper` a temporary
  lib target). Bin-only apps can't be depended on anyway.
- **A-007 · M0-03** · `Unlicense` added to the allow-list (public-domain-style, pulled in by wasm-only transitive
  crates `async_io_stream`, `pharos`, `ws_stream_wasm`). `unused-allowed-license = "allow"` so the list can be a
  superset. Reverse: remove from `deny.toml`.
- **A-008 · M0-04** · Record encoding is `[version byte] ++ postcard(body)` with the version defined per type
  via `Record::VERSION` (all `1`). There is no type-tag byte; the *key/namespace* identifies the record type.
- **A-009 · M0-04** · `Timestamp` is `i64` unix-ms (so pre-1970 EXIF dates work); `Hlc.physical_ms` is `u64`.
  GPS is stored as integer degrees×1e7 (`Gps`) so equality/ordering are exact.
- **A-010 · M0-04** · `DeviceCert`/`AdminCert` carry an opaque `signature: Vec<u8>` (empty until M2-01) and
  `DeviceCert` has a `revoked: bool` (architecture doc has `revokedAt?`; a bool plus the cert HLC carries the
  same information). `AlbumId` is 16 random bytes. `Member.key` is an `EndpointId`.
- **A-011 · M0-04** · `blake3` is a normal dependency of `shoebox-core` (for `Blake3Hash::of_bytes`); the "no async
  runtime / iroh" rule is enforced by `tests/dependency_policy.rs`.
- **A-012 · M0-05** · Tombstone rule. `05-prototype-plan.md` §2 says "tombstones win over edits made at or *after*
  the delete" while the issue says "at or *before*". I implemented the reading both share: **a field edit never
  undeletes a photo** and the edit is **not discarded** (favorite is its own LWW register, so a delete racing a
  favorite yields "deleted + favorited", and the favorite is still there after a restore). Only an explicit
  `restore` (an LWW write of `deleted_at = None`) with an HLC after the delete's HLC brings the photo back; a
  restore with an older HLC (clock skew) loses. Reverse/extend: add a max-register "delete floor" if edits at or
  before the delete must be dropped after a restore.
- **A-013 · M0-05** · Conflicting *immutable* `PhotoRecord` fields (two importers rendering different
  previews/thumbs of the same original) resolve to the lexicographically lowest
  `(original, preview, thumb, taken_at, exif, imported_by)` on every replica, which keeps merge commutative.
- **A-014 · M0-05** · `OrSet` keeps removed tags forever (grow-only). Compaction is deferred; at personal-album
  scale this is small. `Lww` equal-HLC/different-value ties (impossible in practice) resolve by value.
- **A-015 · M0-05** · `merge` on records returns `Result<_, IdMismatch>` instead of panicking when ids differ.
- **A-016 · M0-06** · `shoebox-core` may depend on `futures` and `async-trait` (executor-agnostic traits and
  `BoxStream`); the dependency-policy test now forbids only runtimes (`tokio`, `async-std`, `smol`) and `iroh`.
- **A-017 · M0-06** · The trait sketch in `05-prototype-plan.md` was extended where the issues needed it:
  `LibraryStore::restore_photo`, `deleted_photos`, `album_items`; `StoreBackend::device()`. `SignedDeviceList` /
  `SignedDeviceChange` are `{devices, admins}` / `Device(cert) | Admin(cert)`; device certs merge LWW per device
  by HLC (a revocation is just a later cert with `revoked = true`).
- **A-018 · M0-06** · `AlbumStore::sync_with` was added (the sketch had none) so shared albums can be tested with two
  replicas; M5-01 may reshape it. `PeerAddr` is a backend-agnostic `{id, relay_url, direct_addrs}`; the iroh
  backend maps it to `EndpointAddr`. Invites in the mem backend are validated by a per-inviter secret table; `Viewer`
  and `Contributor` cannot invite, only `Owner` can.
- **A-019 · M0-06** · Purging is replicated with a per-photo purge marker (an HLC, merged by max). A record is dropped
  everywhere if its latest delete/edit HLC is not after the marker, so stale replicas cannot resurrect it, while
  re-importing the same file later is a fresh record that survives. Markers are kept forever (tiny).
- **A-020 · M0-06** · `mark_deleted` on an already-deleted photo is a no-op (keeps the original deletion time);
  `restore_photo` on a live photo is a no-op. `put_holdings` rejects any device other than the store's own
  (`Invalid`), after the read-only check. Every store write observes incoming HLCs so a local write issued after a
  sync always orders after everything seen.
- **A-021 · M0-06** · `album_items` hides soft-deleted/purged photos but not photos that have not synced yet.
- **A-022 · M0-07** · Conformance lives in `shoebox-core::conformance` (feature `testing`), runtime-agnostic (async
  but no executor). A backend implements `Harness` (`fresh`, `backend`, `peer_addr`) and calls
  `conformance::run(&harness)` from any executor; `shoebox-store-mem` ships `MemHarness` behind its `testing`
  feature, so its own test is three lines. Sanity-checked by mutating the mem backend (broken add-wins, thumbs
  counted as copies, HLC ignored in holdings merge): the suite failed each time.
- **A-023 · M0-08** · Real HEIC/RAW/MP4 samples with third-party licenses would have to be downloaded, so the checked-in
  set is **generated from scratch** by `cargo xtask samples` (CC0). HEIC comes from `heif-enc`, video from `ffmpeg`.
  The RAW+JPEG pair's `.dng` is a *synthetic stand-in* (valid TIFF/DNG structure, not decodable by macOS ImageIO), good
  for pairing/extension logic only. **Follow-up for review:** swap in a real, permissively licensed DNG before M1-04
  tests RAW decoding.
- **A-024 · M0-08** · The corpus is tiny JPEGs (48–80 px) so 10k photos generate in <1 s and fit in ~42 MB. Duplicate
  rate is a fixed 5%; seeds are deterministic (SplitMix64). In metadata-only mode (`large`), hashes are stand-ins
  derived from the content seed and `size` is 0. `cargo xtask` is an alias for a *release* build of `xtask`.
- **A-025 · M0-01/M0-08** · The `hello-sync` spike ran on loopback with `MemoryLookup` only: relay, NAT traversal and
  real mDNS behaviour are *not* verified yet (M1-09 / M6-01).
- **A-026 · M0-08** · `jpeg-encoder` (xtask only) is `(MIT OR Apache-2.0) AND IJG`; IJG is permissive but needs
  attribution in shipped binaries, so `deny.toml` carries a per-crate exception instead of allowing IJG globally.

## M1

- **A-027 · M1-03** · Blob protection is modelled as iroh-blobs tags (`lib/`, `pin/`, `album/<id>/`). iroh-blobs only
  deletes through periodic GC (its manual delete is crate-private), so `remove()` drops the library+pin tags and the
  GC reclaims the bytes later (default every 5 min, configurable via `gc_interval_ms`). A shared-album tag blocks GC
  after the library purges a photo: that is the "hook for M5". In-flight downloads hold a temp tag (a race here
  deleted fresh downloads before it was added). `BlobProgress` events are throttled to one per 256 KiB.
- **A-028 · M1-03** · The 2 GB by-reference acceptance check is an opt-in test (`--ignored two_gigabyte`, 31 s in
  release) rather than part of the default suite; the default suite uses a 48 MB file with the same assertion.
- **A-029 · M1-04** · Thumbnails/previews are always JPEG (q80 / q85); `image`'s WebP encoder is lossless-only. `taken_at`
  from `DateTimeOriginal` is treated as UTC unless `OffsetTimeOriginal` is present (EXIF has no zone otherwise).
  Reported width/height are as displayed (orientation applied); for RAW they are the embedded preview's, not the sensor's.
- **A-030 · M1-04** · RAW: the largest decodable embedded JPEG found by scanning for JPEG start markers (first 32).
  Videos: MP4/MOV `moov` parsed for `mvhd` creation time and `tkhd` size; no poster frame (M3-10), no GPS. A JPEG
  without an end-of-image marker is rejected as truncated (decoders otherwise return a partly grey image).
  Decoding is wrapped in `catch_unwind` so a decoder panic surfaces as an error.
- **A-031 · M1-04** · The `heic` feature (libheif-rs) is written but **not compiled or tested**: this machine has libheif
  but no `pkg-config`. Without the feature HEIC is reported as `Unsupported`. 1146 photos/s on the 10k `medium` corpus
  is for its tiny (48–80 px) images; real 12 MP photos will be far slower, so re-measure with real images.
- **A-032 · M1-05** · The docs backend keeps the *replicated records* as iroh-docs entries (layout in
  `shoebox-store-docs::codec`, values `[v1] ++ postcard`) and answers queries from a merged in-memory `LibraryState`
  rebuilt at open and refreshed by (debounced) rescans. This deviates from the issue's "secondary key by taken_at":
  the timeline index is in memory, not in docs. Cost: RAM proportional to library size and an O(n) metadata scan per
  refresh. **M1-15 must measure both**; incremental refresh from `InsertRemote` events is the obvious optimisation.
- **A-033 · M1-05** · Each device writes its merged view of a record under its *own author*; readers merge every author's
  entry with `core::merge`. Purging deletes only this author's entry; other authors' stale entries stay in docs
  (ignored thanks to the purge marker) until those devices purge/merge too. Entries whose key does not name their own
  record are rejected.
- **A-034 · M1-05** · `StoreBackend` got `downgrade(cap)`; `LibraryCapability` and `InviteTicket` got an opaque `backend`
  payload (docs: the postcard `iroh_docs::Capability` / a `DocTicket` string). A read-only library is a real read-only
  namespace key. A viewer who joins a shared album holds a read-only capability and therefore cannot publish its own
  membership entry: it is *not listed* in `members()` on other replicas (the mem backend lists it).
- **A-035 · M1-05** · The device identity written into records ("logical device") is decoupled from the iroh node id so the
  conformance suite can use small ids; in production `DocsBackend::for_node` uses the node's endpoint id.
- **A-036 · M1-05** · iroh-docs 0.101 on iroh 1.4 reports `Failed to close connection` from the *closing* phase of an
  otherwise successful sync; `sync_with` treats exactly that error as success (any other `SyncFinished` error fails).
  Also required: `start_sync(vec![])` on every opened namespace, otherwise incoming syncs abort with `NotFound`.
- **A-037 · infra** · The dev profile uses `debug = "line-tables-only"` (none for dependencies). Full debuginfo for the iroh stack
  grew `target/` to 13 GB and filled this machine's disk mid-session; the lean profile keeps it ≈2 GB. Reverse in the root `Cargo.toml`
  if you need full debuginfo for a dependency.
- **A-038 · M1 ordering** · The dev CLI (M1-07) is implemented *after* M1-08…M1-13: its `join`, `peers`, `holdings` and `fetch`
  commands and the demo script need sync, discovery, presence, holdings and the fetcher. Everything else follows the plan order.
- **A-039 · M1-06** · The node-level orchestration lives in `shoebox_node::LibraryNode<L: LibraryStore>`, generic over the backend
  (the docs backend depends on `shoebox-node`, so `Node` itself cannot own a store). Tests use the mem backend for speed.
- **A-040 · M1-06** · Imported photos start with their `favorite`/`deleted_at` registers at `Hlc::GENESIS`, so importing a file another
  device already favorited or deleted never overrides that edit. If a purge marker suppresses such a record, the import is retried
  with fresh registers (a deliberate re-import of a purged photo). Videos use the original as preview/thumb until M3-10.
  Analysis (hash + renditions) runs even for files already in the library, which keeps the pipeline simple but wastes work on
  re-imports; a hash-only fast path is a possible optimisation. Hidden files and symlinks are skipped during folder walks.
- **A-041 · M1-06** · Watched folders are debounced 400 ms after the last event, persisted in `config.toml` (kept across restarts even
  when the caller passes a fresh `NodeConfig`), and rescanned on start so files that arrived while closed are imported.
- **A-042 · M1-08** · `LibraryNode` runs a sync manager: it syncs a known peer when it connects, when added, and when due
  (healthy peers every 5 min as a safety net behind iroh-docs' live gossip sync; failures back off 1 s → 60 s, doubling),
  and emits `SyncProgress {Started | Finished | Failed(reason) | Retrying{in_ms}}` per peer. `LibraryTicket` is a dev-only
  string (`shoebox-library1…`: capability + peer addresses + a checksum) that M2 pairing replaces.
  **Not verified here:** "two desktops on different networks, one via relay". Measured locally instead: 10,000 records written on
  one node converge on a second within 1.4 s of the last write (loopback; the writes themselves took 12.5 s).
- **A-043 · M1-05/08** · Lesson recorded: iroh-docs delivers events through bounded channels and a subscriber that stops draining
  stalls the whole engine (including the queries a rescan needs). The docs backend therefore drains events in a dedicated task
  that only signals "dirty" and rescans in another. `sync_with` also keeps rescanning (max 10 s) until no entry is waiting on its
  content, because `PendingContentReady` does not guarantee all content is local (this caused a rare flake in the 3-node test).
- **A-044 · M1-09** · mDNS is on by default and off in `NodeConfig::local_only` (tests/explicit LAN-less use). `RelaySetting::Custom(url)`
  is parsed at open (a bad URL fails fast) but not contacted until used. `peer_info` classifies the in-use addresses
  (private/link-local/loopback → LAN, other IPs → Direct, else Relay) and reports the last ping RTT and last connection time.
  mDNS between two nodes on this machine is tested; **not verified:** with the internet physically unplugged, and the manual
  "relay → direct after hole punching" check (needs two real networks).
