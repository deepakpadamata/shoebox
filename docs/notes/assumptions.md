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
- **A-045 · M1-10** · Presence is per library: topic = blake3(library id ‖ root key), so read-only and write devices share it and other libraries
  do not. Messages are unauthenticated in M1 (signed with device certs in M2). Defaults: heartbeat 10 s, offline after 35 s (the issue's
  budget is 60 s). A `NewItems` nudge makes the receiver sync with the announcing device (only if it is a known library peer); imports announce
  automatically. `start_presence` must be called explicitly after `with_capability`.
- **A-046 · M1-11** · Added a replicated, last-writer-wins `settings` facility to `LibraryStore` (`setting`/`put_setting`, keys `[a-z0-9._-]{1,64}`,
  opaque byte values) with its own conformance scenario; the redundancy target is the setting `redundancy.target` (one byte, default 2, ≥1).
  Copy counts include devices that are currently offline (their data exists). `at_risk` over 100k photos takes ~22 ms in a release build
  (the debug-build test uses 10k photos because *building* a 100k library in debug takes over a minute).
- **A-047 · M1-05/08/11** · Hardening of the docs backend found by stress runs (each run of the docs test binaries, 25–40×, before/after):
  (1) `SyncFinished` carries no reliable outcome in iroh-docs 0.101/iroh 1.4: nearly every sync reports "Failed to close connection",
  including complete ones and ones where two syncs between the same peers overlapped and *no data moved*. So `sync_with` now asks the peer for
  an order-independent **digest** of its namespace (`shoebox/doc-digest/0`, a tiny protocol served by every node) and syncs again until the
  digests match (≤ 6 rounds, then it accepts — live writes can keep digests moving). It costs one O(entries) listing per round on each side.
  (2) A sync request is ignored while another sync with that peer is running, and that one may predate the data: only a sync that *started
  after the request* counts, and a stale finish triggers a fresh request. (3) iroh-docs fetches an entry's content once per sync run and
  never retries, so the replica fetches content that has been missing for 750 ms itself, from peers it has exchanged entries with, retrying
  every 4 s and rescanning every 500 ms while anything is pending. Flake rate went from ~15% to 0 in 90 consecutive runs.
- **A-048 · M1-12** · The fetch policy is `shoebox_core::policy` (pure): Keeper = all; Phone = thumbs+previews, plus an original only when pinned or
  it is this phone's own capture with fewer copies than the redundancy target; Viewer = thumbs+previews. Shared-album items: previews, and the
  original (or the location-stripped `shared_hash` copy) only for Keepers with "keep originals". Videos use the original as thumb/preview until
  M3-10, so a viewer *does* have to be able to open them on demand, but never pulls them in the background (the placeholder is only ever
  planned as an original). Queue order: all thumbs → all previews → all originals, newest first. Holder choice: best network path (LAN >
  direct > relay > unknown), then most recently seen, then id; failures fall through to the next holder, and a failed blob cools down for `retry`.
- **A-049 · M1-12** · Pins are per-device and live in `config.toml` (`pinned_originals`), not replicated. iroh reports download progress as
  "bytes available" (local + received) and requests only missing ranges, so resumption is verified by the first progress event starting at the
  already-held half (a 32 MB blob with its first 16 MB pre-imported). `missing_blobs` lists the whole library each pass (O(n) records cloned),
  fine for the prototype but a candidate for an incremental index at 100k+ photos (M1-15 will say).
- **A-050 · M1-13** · Blob reclamation is *reconciliation*, not bookkeeping at purge time: a purge removes the record (on every device), so each device
  independently releases its `lib/`+`pin/` protection on blobs that no live or soft-deleted record references and retracts the holdings; iroh-blobs'
  periodic GC then deletes whatever has no protection left. This works however the record vanished (local purge, replicated purge marker, crash in
  between) and leaves `album/<id>/<hash>` protections alone (the M5 shared-album hook; verified: an album-referenced preview survives the purge).
  An `ingest` RwLock keeps a reconcile from releasing a blob whose record is not written yet. `start_maintenance` runs purge+reconcile hourly and
  within a debounce (default 5 s) after any delete/purge change, on every device. A purge is performed by whichever device's clock passes the 30-day
  mark first; markers are monotonic so concurrent purges agree.
- **A-051 · M1-13** · Found while adding fast blob GC to the testkit: iroh-docs + iroh-blobs GC can delete a just-written entry's content on the
  *writer* (raw iroh-docs, no networking, GC every 100 ms: 10 of 3000 entries lost their blob). The replica now writes content itself
  (`add_bytes` + `set_hash`) and holds a temp tag for 60 s after the entry exists. The testkit clusters run GC every 200 ms, so every multi-node
  test doubles as a regression test for this. Production GC defaults to every 5 min, which makes the race rarer but not impossible without the hold.
- **A-052 · M1-13** · Blob GC is gated on node *activity*: the node records when it last imported, fetched, wrote blobs/docs entries or synced, and the
  GC protect callback aborts a pass if that was within `gc_quiet_ms` (default 30 s; testkit 400 ms). iroh's snapshot-then-sweep GC can otherwise
  delete blobs created or referenced mid-pass (reproduced as lost entry content and lost library records under a 200 ms GC). Also verified
  experimentally: a *temp* tag keeps a **partial** blob alive through GC while held (a named tag does not), so a failed fetch now leaves a temp tag on
  the partial blob for up to an hour to preserve progress for the retry (in-process only: after a restart GC discards partials).
- **A-053 · M1-07** · `shoebox-dev` is a *one process per data dir* tool: `run` keeps a device online; the other commands open the dir briefly, so
  they cannot run against a dir that `run` is using (a second `--dir` plays the second device; M2's keeper control socket removes this limit).
  One-shot commands that change holdings (`fetch`) sync with their peers before exiting, otherwise nobody would learn about the new copy. Device
  state lives in `<dir>/dev.json` (name, role, the library ticket, known peers) and the node under `<dir>/node/`. `--local-only` = no relay, no mDNS.
  `LibraryNode::sync_now` now waits for a sync that is already in flight and returns its outcome (the CLI's `join` returned an empty library because
  it exited while the background manager's sync was still running). The root key in `init` is a placeholder derived from the device id (M2 adds real identity).
- **A-054 · M1-14** · The Loro spike keeps the *same* merge/query engine (`LibraryState`) and the same per-author key/value layout as the docs backend (the
  codec moved to `core::entry_codec` and is shared), so the benchmark compares the replication layers rather than two query engines. Its sync is
  simulated in-process with real Loro export/import (no network), shared albums are not implemented, and the conformance suite gained `run_only` /
  `SCENARIOS` to run a subset. The `loro` cargo feature is on by default for the crate; depend with `default-features = false` to leave it out.
- **A-055 · M1-15** · Decision #7 recorded as **hybrid: personal library on Loro, shared albums on iroh-docs** (issue M1-16, +16 dev-days, planning README
  re-estimated: M1 now 16 issues, total 82 issues / 197 dev-days / ~47 weeks). I made the *recommendation*; the switch itself is **not** done and should be
  reviewed first (the benchmark has caveats: Loro spike has no transport, no phone-class device was available, iroh-docs ≥100k fails in this setup). Work on
  M2+ continues against the `LibraryStore` trait, which is unaffected either way. The benchmark also surfaced a quadratic `Effect::touch` (fixed), that per-photo memory
  (~5 KB) is shared by both backends and needs work for phones, and that holdings should be published for originals only.
- **A-056 · M2-01** · Identity = ed25519 (dalek 3, strict verification). Signatures cover a domain-separated, library-bound message
  (`shoebox/{admin,device}-cert/v1` + library id + postcard of the cert fields), so a cert cannot be replayed into another library. A device cert signed by the
  root has `added_by` = the root public key bytes. Authority is judged **at the cert's own HLC time**: the signer needs a root-signed admin cert that had
  started and not expired by then and must not have been revoked earlier; the chain is replayed in HLC order, so the result never depends on arrival order.
  `AdminCert` gained `expires_at: Option<Timestamp>` (golden bytes updated). A revoked admin's earlier certs stay valid; later ones do not.
- **A-057 · M2-01** · `LibraryState` verifies certs only when it knows the root (`with_root`; all three backends pass `cap.root`/`cap.library`). Local API calls
  (`apply_device_change`) reject bad signatures (`BadSignature`) and unauthorised signers (new `StoreError::NotAuthorized`). **Replicated** entries are kept
  and re-judged on every change, because a device cert can arrive before the admin cert that authorises it; only broken signatures are dropped. Consequence:
  anyone holding write access to the library space can park unauthorised-but-correctly-signed certs in memory (bounded by what they can write); they never
  affect membership. Without a root (unit tests only) the old "non-empty signature" stub applies.
- **A-058 · M2-01** · `shoebox-dev init` creates a random root key, self-issues an admin cert and a device cert, and keeps the root seed in `dev.json`
  (dev tool only; real devices never store it, see M2-02). `shoebox-dev join` does not yet obtain a cert: pairing (M2-04) does that.
- **A-059 · M2-01** · Known flake: `shoebox-store-docs` `autosync::sync_now_waits_for_a_sync_already_in_flight` (300 records, `fast()` 10 s sync timeout) fails
  intermittently — 1 in 6 runs on the pre-M2 tree (verified with `git stash -u`), more often on a nearly full disk. It is a sync that hangs inside iroh-docs, unrelated
  to identity. Not fixed; re-run before treating it as a regression. Worth a proper look if it appears in CI.
- **A-060 · M2-02** · The kit holds the *root key* (24 words = its 32-byte seed as BIP39 entropy, English list; typed phrases are case/space-forgiving) plus library id and
  creation date in a printable text file (`RecoveryKit::to_text/from_text`). `LibraryNode::create_identity(root, …)` returns the kit and keeps nothing (the issue's
  `Node::export_recovery_kit()` cannot exist because the root is never stored; export happens once, at creation). `restore_from_kit(phrase)` verifies the phrase against the
  library's root key before writing anything, then certifies the new device as admin member. **Gap (acceptance "rejoins and sees all photos once a keeper is reachable")**:
  the kit does not contain the library's *space capability*, so a device that lost everything cannot yet fetch data; this needs a root-proof handshake in which a surviving
  member/keeper hands the capability to a device that signs a challenge with the root key — scheduled with the pairing protocol (M2-04). `recovery_kit_saved` is the
  replicated setting `recovery_kit_saved` (`[1]`).
- **A-061 · M2-03** · Enforcement is by **connection gating**: `Node::set_allowed_peers` (the non-revoked device list; empty set once this device itself is revoked) is applied in
  iroh's `before_connect`/`after_handshake` hooks, so it covers every protocol (docs, blobs, gossip, ping) and closes live connections of newly refused peers. Pairing
  (`shoebox/pair/0`) stays open to unlisted devices. Nothing is refused while no device list exists yet, so a fresh joiner can sync; a library whose first
  certs have not arrived is therefore unprotected for that moment. A revoked device that learns of its own revocation gets `Membership::Removed` +
  `NodeEvent::RemovedFromLibrary`; one that never hears (peers cut it off first) just cannot connect. **Not done / split into M2-03b:** capability rotation (the revoked device
  keeps the namespace secret), and per-record filtering of writes authored by a revoked device (unnecessary while it cannot connect, and not possible to enforce
  per record because registers merge by LWW without an author signature). The 100k-record rotation criterion moves with it.
- **A-062 · M2-04** · Pairing as built: ticket = `{inviter addr, 32-byte secret, expiry, role}` (`shoebox-pair1` + hex + checksum, same shape as the library ticket; this string is what a QR
  encodes). The secret authenticates the exchange (keyed-BLAKE3 proof over both ids and a joiner nonce; the ticket is found by a key-derived 8-byte id); the six-digit code is a keyed hash of
  secret, ids and both nonces — people compare it, and *either* side declining aborts before anything is written. The inviter consumes the ticket on a valid proof (single use), expiry
  uses the node clock (default offered 5 min by the caller), and failed lookups are rate-limited globally (30/min, then everything is refused for a minute; per-ticket proof
  failures burn a ticket after 5, though a ticket id cannot be guessed without the secret). The grant carries the device cert, the inviter's root-signed admin cert and the library
  capability; the joiner verifies the chain to the capability's root key before trusting it. The role is fixed by the ticket (a camera device scanning a QR gets what the QR offers).
  Admin rights are not granted by pairing. **Verified: loopback only.** "Through a relay" is unverified (no relay available here; the protocol is plain QUIC streams so nothing
  relay-specific is expected). The `shoebox-dev` CLI does not use pairing yet — the keeper CLI (M2-06) will. Recovery-with-root-key still lacks its capability handoff (A-060).
- **A-063 · M2-05..07** · Keeper: one daemon per storage dir (`keeper.lock` via `File::try_lock`, so the **workspace MSRV is now 1.89**; clippy msrv follows). Node data lives directly under
  `--storage` (`keys/endpoint.key` 0600, `blobs/`, `store/`, `keeper.json`, `control.sock` 0600, `keeper.lock`). `keeper.json` stores the library ticket string (capability + peers
  from the pairing grant), so a restart rejoins and the fetcher resumes (tested: stop right after pairing, restart, all originals arrive). `pair` runs *inside* the daemon over the
  socket (the same node must hold the endpoint key that the cert is issued to) — pairing an offline keeper is not supported; start `run` first. A paired keeper gets role Keeper and
  no admin rights, so `devices revoke` on it fails with "only an admin device…" until an admin-granting flow exists (not in M2). The 5g/5i mockups are not in the repo; terminal output
  is my reading of the content list in the issue (library counts, at risk, devices with path/latency, shared-album count), documented with its JSON schema in `docs/keeper-cli.md`.
  `status` computes `held_originals` by probing the blob store per photo (fine for tens of thousands; should become an index if status is polled on very large libraries). Service files:
  systemd user unit (default) or system unit (+ optional dedicated `User=`), launchd plist in `~/Library/LaunchAgents`; `service install` writes the file and prints the
  enable command rather than invoking `systemctl`/`launchctl` itself (not verifiable here). **Unverified:** the acceptance test "fresh Raspberry Pi OS: install → reboot → syncing" (no Pi, no systemd on this Mac).
