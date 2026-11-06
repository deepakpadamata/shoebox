# Assumptions log

Reasonable assumptions made while implementing the issues (IDs like M2-02), recorded for review.
Each entry: **ID** · issue · assumption · why · how to reverse.

**Active** entries still matter for the remaining work, grouped by area. **Settled** entries describe decisions
that are built and covered by tests; they are kept for reference. New entries go under Active; move one to
Settled once nothing remaining depends on it. IDs never change.

## Active

### iOS, HEIC and phones

Android is built and checked on an emulator; iOS is not built. iPhones shoot HEIC, which import and location stripping do not handle yet.

- **A-031 · M1-04** · The `heic` feature (libheif-rs) is written but **not compiled or tested**: this machine has libheif
  but no `pkg-config`. Without the feature HEIC is reported as `Unsupported`. 1146 photos/s on the 10k `medium` corpus
  is for its tiny (48–80 px) images; real 12 MP photos will be far slower, so re-measure with real images.
  **Update (I1-02, 2026-10-31):** HEIC and AVIF are decoded by the platform where it has a decoder, and by a
  built-in one where it has not.
  - **macOS:** ImageIO (`heic_macos.rs`, registered in `AppCore::open` through `shoebox_media::set_platform_decoder`).
    HEIC and AVIF; tested on a plain and a rotated HEIC and an AVIF.
  - **Linux keeper, Windows and Linux apps:** `heic-static`: libheif 1.23.1 and libde265 1.0.16 built from source
    (`vendor/libheif-sys`, a copy of the crate whose `build.rs` has a `static-decoders` feature, and
    `vendor/libde265`, the release tarball v1.0.16, SHA-256 `b92beb6b53c346db9a8fae968d686ab706240099cdd5aff87777362d668b0de7`,
    docs, tools and autotools files left out) and linked statically. HEVC only: **no AVIF there** (no AV1 decoder;
    the file is skipped with "this build cannot decode it"), and none of libheif's encoders or JPEG codecs. The crate's own
    `embedded-libheif` is not used: it builds only libheif and links whatever decoders and encoders (x264 and x265 are
    GPL) the machine has. Cost: +1.9 MB on the 60.3 MB x86_64 musl keeper. **Verified:** the x86_64 musl keeper is a
    static ELF and its test binary ran the HEIC decode in a Linux container (13 of 13 `analyze` tests). **Not
    verified:** Windows (MSVC), the aarch64 musl keeper, the Linux AppImage, and the release workflow's new zigbuild step.
  - **Android:** the system `ImageDecoder` (`HeifDecoder.kt`, called from `heic_android.rs` over JNI and read back with
    `AndroidBitmap_*`; sRGB, software bitmap, one decode at a time). HEIC needs Android 9, AVIF Android 12; on an older
    one the file is skipped as unsupported. **Verified** on an Android emulator and on a Samsung S25 (HEIC import and
    viewing; the viewer shows the preview, as a web view cannot open a HEIC original). **Not verified:** AVIF on the S25, a rotated HEIC or Display P3 colour on Android, and
    the background workers' registration path.
  - Every HEIC/AVIF is first checked for a complete box structure, because ImageIO decodes a cut-off file into a partly
    grey picture. To update libde265 or libheif: replace the folder, rebuild, and update `scripts/licences.py` and this entry.
- **A-080 · M5-03 location** · `strip_location` zeroes the GPS IFD (and its values) and removes its pointer from IFD0 for JPEG, drops `eXIf` and GPS-bearing XMP chunks for PNG, blanks `exif:GPS*` XMP properties, and turns `©xyz`/`loci`/QuickTime location atoms of MP4/MOV into `free` boxes. Pixel/sample data is byte-identical (tests compare the bytes after the image data and the decoded pixels). *Not handled:* HEIC/HEIF/AVIF (refused, never shared with its location when "remove" is chosen), GPS inside an embedded EXIF thumbnail or a maker note, other containers. The stripped copy becomes the item: its id and `original`/`shared_hash` are the copy's hash, so members never learn the hash of the GPS original (they could otherwise fetch it from the contributor). Previews and thumbnails are re-rendered by the image pipeline and carry no metadata. The prompt counts only photos whose library record has a GPS fix; videos are stripped when "remove" is the album's setting even though their location is not in the record. Stripping reads the whole original into memory (fine for photos, heavy for long videos).
- **A-072 · M3-07** · The recovery phrase exists only in the `create_library` result and the onboarding screen's memory: the root key is dropped on the Rust side and nothing is stored, so a reload
  before "Continue" loses it (the library exists; the kit was never saved). The reminder banner can only ask the user to confirm they saved it. Mockup 1d's import/role step is replaced by the
  Import button in the toolbar. The `AppCore` `recover` path (phrase + `shoebox-peer1…` link) is in the UI; automatic discovery of a surviving device is not.
- **A-074 · M3 gaps closed** · (1) *Video posters:* the library record of a video has no separate thumbnail (it points at the video), and changing records is out of scope, so each device makes its own
  poster: the UI loads the original (byte ranges) in a hidden `<video>`, grabs a frame at ~10 %, and `set_poster` stores a JPEG (<= 2 MB, JPEG-checked) under `<data dir>/posters/`, served as blob kind
  `poster`. Device-local and never replicated; a device that only has the thumbnail-less record (no original here) shows the neutral tile. Needs a webview that can decode the codec (tested with the
  sample MP4/MOV in a Chromium pane; WKWebView/WebView2 codec support not checked). (2) *Albums:* `rename_album` is an LWW name write that leaves cover and items alone; the album view has Rename and
  "Remove from album". Deleting an album is not offered: the personal-album CRDT has no tombstone. (3) *Dev libraries:* if the data dir has no `app.json` but a `shoebox-dev` `dev.json`, the app adopts it
  (same node dir, same device key, same library; the dev root seed is ignored) and writes `app.json`; a corrupt `dev.json` is an error. Verified against a library made by the real `shoebox-dev init`.
  (4) While testing this a race in the pairing prompt showed up (the UI can answer before the question is registered): answers are now remembered (`PairingSlot::Early`).
- **A-075 · M4 (Android)** · (1) Backup state (`backup.json`) tracks pending and imported asset ids by MediaStore generation; an edited photo (new modification time, same id) is imported again, a photo deleted from the roll stays in the library; identical bytes under another id count as a duplicate. (2) Device secrets are wrapped with an Android Keystore AES-GCM key; earlier plain key files are migrated and removed, and `allowBackup` is off so they never reach cloud backup. (3) The status bar and gesture bar insets reach the page as `--safe-top/--safe-bottom`. (4) `sync_burst` returns at once with "no other devices are paired yet" when there are no peers; notifications fire at most once per 24 h.
- **A-076 · M4 headless sync** · WorkManager's `BurstWorker` calls `NativeBridge.runBurst` (JNI) which opens the same app core as the UI, imports new photos, runs a bounded burst and exits; it skips when the UI process holds `app.lock`. Android decides when it runs, so timing is best-effort; no foreground service, so a very large first transfer relies on the app being open (keep-awake). A debug-only `BurstNow` receiver exists for testing and is not in release builds. Notification posting was not seen on a device.
- **A-077 · M4-07** · Joining by QR uses `tauri-plugin-barcode-scanner` (mobile only; camera permission). The scan was built but not run, since the emulator camera shows no QR code; pasting the ticket remains.

### Phone memory and scale

The 300 MB at 100k photos target for phones is not met (about 500–700 MB).

- **A-100 · previews next to originals (owner's request, 2026-10-28)** · A device other than a keeper that holds an
  original the app shows as it is (JPEG, PNG) does not fetch its preview, and lets one it has go once a *keeper*
  confirmed (`shoebox/has/0`, within 7 days) that it holds it (`LibraryNode::release_previews`, hourly maintenance).
  The viewer shows the original; a request for the preview is answered with the original (`blob.rs`). Previews are
  about a quarter of the photo originals' size, so a phone saves that much for every original it holds. Only a keeper
  counts as the other copy: two phones holding the original could each rely on the other and both drop it, and a
  preview is not reproduced byte for byte by another build. HEIC and raw keep their previews (not every web view
  shows them); a preview a shared album uses stays. Freeing an original fetches its preview back first, and keeps the
  original when that fails. Reverse: drop `holds_original` from `PlanContext` and the maintenance call.
- **A-101 · rendition encoder (2026-10-28)** · Thumbnails and previews are baseline JPEG with 4:2:0 chroma from
  `jpeg-encoder` (13% smaller, 2.6× faster to encode than the `image` crate's 4:2:2). Not with optimized Huffman
  tables: zune-jpeg 0.5 decodes those 4:2:0 files to the wrong colours (macOS decodes them correctly), and Rust decodes
  whatever is imported. Only new imports get the new renditions. Reverse: `render::encode_jpeg`.

- **A-102 · choosing what backup takes (owner's report, 2026-10-28)** · With access to all photos, backup took every image
  and video MediaStore lists: downloads, screenshots, messaging apps' pictures. `BackupPolicy.scope` says what it takes:
  `camera` (the default, also for a `backup.json` written before it: files under `DCIM`, except `DCIM/Screenshots` and
  screen-recording folders, where Samsung keeps screenshots), `all` (every folder, new ones included) or `folders`
  (exactly the chosen folders, by absolute path, so internal storage and a card are separate). *Choose folders* lists
  every folder of the roll with its count, camera folders first (`AppCore::backup_folders`, a full scan grouped by each
  file's folder: no platform code, so the fake photo source tests it). Decided in `backup.rs`; a photo already in the
  library still follows its edits and deletions whatever the scope; narrowing the scope removes nothing, and any change
  forgets the scan token so the next scan sees every folder. A photo with no readable path stays in scope (reported and
  retried as before). Photos the person adds with ＋ are never filtered. Not covered: a camera folder that appears after
  folders were chosen is not added by itself (`camera` and `all` follow new folders). `desktop-dev-server
  --fake-camera-roll <dir>` serves a folder as the gallery to try this in a browser.
- **A-103 · delete now from Recently Deleted (owner's request, 2026-10-28)** · *Recently Deleted* has *Delete N now*
  (the selection) and *Delete all now*, after a confirmation. `LibraryStore::purge_deleted(ids)` purges only photos that
  are soft-deleted (live and unknown ids are ignored) with the same purge marker as the 30-day purge, so it replicates
  to every device and keeper with no format or protocol change, and a stale copy cannot bring the photo back; a restore
  on another device that is HLC-later than the marker wins, as for the 30-day purge. `LibraryNode::purge_deleted` then
  releases the blobs nothing references. Gallery files and watched-folder files are never touched (A-092): only the
  library's record and its own copies go. Any device that can write the library can do it, like deleting.
- **A-104 · video posters in the library (owner's request, 2026-10-29)** · A video's `thumb` was its original (a
  placeholder) and each device made and kept its own poster frame. Now a poster becomes the video's `thumb`
  (`LibraryNode::set_video_poster`, a JPEG of at most 480 px, about 15 KB) and replicates, is fetched and is kept like
  a photo's thumbnail. `PhotoRecord::merge` ranks a real thumb above the placeholder (then the lowest, as before), so
  the first poster any device adds wins everywhere; no format change, and a build without this rule still converges
  on the placeholder it prefers among its own replicas. Android makes posters during backup with the system
  thumbnailer (`PhotoSource::video_frame`, at most 60 a run); elsewhere the app grabs a frame when it shows a video no
  device has a poster for. Posters in an earlier build's `posters/` folder move into the library at start, once.
- **A-105 · search by facts and places (S1-02…S1-05, 2026-10-30)** · Dates in a query are read in the
  asking device's time zone (the app sends its offset; the keeper and `shoebox-dev` use UTC unless given
  `--tz-offset`). `taken_at` is UTC when the photo's EXIF has an offset, else the camera's local time stored as if
  UTC, so a photo near midnight can fall on the neighbouring day. Seasons are three calendar months, checked in the
  photo's hemisphere (from its GPS, else the device's, today always northern); a season's year is the year it
  starts in (southern summer 2019 is Dec 2019–Feb 2020). Holidays are fixed rules (Christmas 24–26 Dec, Easter
  Good Friday to Easter Monday, Thanksgiving the US one) and Diwali is a table for 2000–2030, matched a day either
  side. Screenshots are photos without a camera that are PNG or exactly a common screen size; selfies are photos
  whose lens says "front"; live photos are recorded since F2.10 (see A-109). Label words are suggested but stay in the free text,
  so they rank rather than filter. A name that is several kinds at once is a person first, then an album, a place,
  a camera. Places come from GeoNames `cities5000` (70k places, 1.9 MB): the nearest town within 25 km, plus the
  biggest city (≥ 100k people) within 15 km, and the nearest town's region and country within 200 km; no borders,
  so a photo near a border can get the neighbouring country.
- **A-106 · search models (S1-01, 2026-10-30)** · Chosen on licences alone; nothing is measured yet (no Pi 5 or
  S25 at hand, and too little disk on the development Mac to fetch and convert the weights). See
  [search-models.md](search-models.md). Until the measurements in S1-01 exist, S1-06 onwards is built against an
  `Encoder` trait with a deterministic test encoder, and the real encoders are optional.
- **A-032 · M1-05** · The docs backend keeps the *replicated records* as iroh-docs entries (layout in
  `shoebox-store-docs::codec`, values `[v1] ++ postcard`) and answers queries from a merged in-memory `LibraryState`
  rebuilt at open and refreshed by (debounced) rescans. This deviates from the issue's "secondary key by taken_at":
  the timeline index is in memory, not in docs. Cost: RAM proportional to library size and an O(n) metadata scan per
  refresh. **M1-15 must measure both**; incremental refresh from `InsertRemote` events is the obvious optimisation.
- **A-040 · M1-06** · Imported photos start with their `favorite`/`deleted_at` registers at `Hlc::GENESIS`, so importing a file another
  device already favorited or deleted never overrides that edit. If a purge marker suppresses such a record, the import is retried
  with fresh registers (a deliberate re-import of a purged photo). Videos use the original as preview/thumb until M3-10.
  Analysis (hash + renditions) runs even for files already in the library, which keeps the pipeline simple but wastes work on
  re-imports; a hash-only fast path is a possible optimisation. Hidden files and symlinks are skipped during folder walks.
- **A-049 · M1-12** · Pins are per-device and live in `config.toml` (`pinned_originals`), not replicated. iroh reports download progress as
  "bytes available" (local + received) and requests only missing ranges, so resumption is verified by the first progress event starting at the
  already-held half (a 32 MB blob with its first 16 MB pre-imported). `missing_blobs` lists the whole library each pass (O(n) records cloned),
  fine for the prototype but a candidate for an incremental index at 100k+ photos (M1-15 will say).
- **A-055 · M1-15** · Decision #7 recorded as **hybrid: personal library on Loro, shared albums on iroh-docs** (issue M1-16, +16 dev-days, planning README
  re-estimated: M1 now 16 issues, total 82 issues / 197 dev-days / ~47 weeks). I made the *recommendation*; the switch itself is **not** done and should be
  reviewed first (the benchmark has caveats: Loro spike has no transport, no phone-class device was available, iroh-docs ≥100k fails in this setup). Work on
  M2+ continues against the `LibraryStore` trait, which is unaffected either way. The benchmark also surfaced a quadratic `Effect::touch` (fixed), that per-photo memory
  (~5 KB) is shared by both backends and needs work for phones, and that holdings should be published for originals only.
- **A-066 · M1-16** · Loro production backend decisions: (1) *Access control* is the signed-record model: every value carries an ed25519 signature by its author (map-key prefix = first 8 bytes of the
  author's id), importers accept a record only if its author holds a certificate and the role allows that kind of record (viewers may only publish their own holdings); certificate entries are
  self-authenticating through the chain. Residual risk, documented not fixed: a peer can still overwrite another author's *Loro map key* with a bad value (the importer rejects it, but Loro's
  document now holds the bad value, so a restart from disk would lose that record until a good copy re-syncs). (2) The library is "armed" once any device is certified: from then on unsigned
  records are refused by signing replicas; before that, signed records from not-yet-certified authors wait (bounded at 50k) until the certificate arrives, so an uncertified library does not
  converge — creating a library must go through `create_identity`/pairing (the dev tool does). Revoked devices' *existing* records stay accepted (history); only new connections are refused (M2-03).
  (3) Persistence = snapshot + framed update log with group commit (fsync at most every 200 ms; a crash can lose that much *local* work, never remote data) and torn-tail truncation;
  compaction rewrites a full snapshot (history is not trimmed: Loro keeps overwritten values, so the file grows with edit churn — shallow snapshots were not used because peers that lack older
  history could no longer import updates). (4) Transport = one `shoebox/loro/0` stream, version vectors both ways, repeated up to 4 rounds until the vectors match; local writes are pushed by
  the node's sync manager (new `LibraryStore::replicates_by_push`, with a per-peer dirty flag so nudges inside the debounce window are not lost). (5) `LoroBackend::new` (in-process, unsigned)
  remains for conformance and benchmarks; the conformance suite passes on it, and fetcher/holdings/revocation/pairing/free-up/keeper tests now run on `HybridBackend`.
  (6) Not done: phone-class memory work (A-066 note in `store-benchmark.md`), publishing holdings for originals only (the fetcher still publishes thumbs/previews too).
- **A-073 · M3 (UI behaviour)** · The timeline loads every page (2,000 per request) into memory and lays out by day; it reloads (debounced) on `LibraryChanged`. Fine for tens of thousands of photos,
  not yet incremental for 500k (a windowed fetch is the next step). Delete = "Recently Deleted" (restorable, purged after 30 days by the node's maintenance); there is no manual purge. Video tiles
  show a neutral tile (no poster frame, M3-10). The export names files `<date>_<time>_<id8>.<ext>` because original file names are not stored in the library.

### Release and CI

No remote exists, so CI and the release workflows have never run.

- **A-001 · M0-01** · `shoebox-core` and the other crates use one workspace version (0.1.0 since R1-01; the Tauri app and UI follow it) and `publish = false`.
  Why: nothing is released yet. Reverse: bump in `[workspace.package]`.
- **A-002 · M0-02** · `repository` is `https://github.com/deepakpadamata/shoebox` (set in R1-13; an `example.invalid`
  placeholder before the remote existed). The About screen's source offer and the keeper's `licences` show it.
- **A-006 · M0-03** · CI cannot run yet (no remote), so the workflow in `.github/workflows/ci.yml` is unexecuted;
  its steps were reproduced locally (`cargo fmt/clippy/test`, `cargo deny check licenses sources`,
  `scripts/check-no-app-deps.py`). The "deliberate AGPL dependency" acceptance check was verified locally
  with a registry AGPL crate (`lemmy_utils`) in `shoebox-core` → `cargo deny` failed; reverted.
  Note: cargo-deny skips *path* crates as "private", so only registry deps are covered by the allow-list; edges
  from libraries to `apps/*` are covered by `scripts/check-no-app-deps.py` (verified by giving `keeper` a temporary
  lib target). Bin-only apps can't be depended on anyway.
- **A-065 · M2-09** · Release automation is written but never ran: `release.yml` (tag `keeper-v*`, since R1-14 one `v*` tag for every download, see `docs/notes/release-signing.md`; musl x86_64 natively, aarch64 through `cross`, macOS universal via `lipo`; SHA256SUMS signed
  with minisign from repo secrets) and `scripts/install-keeper.sh` (verifies the *signature of SHA256SUMS* then the tarball checksum; refuses to install without `minisign`). The
  project public key is a placeholder (`MINISIGN_PUBKEY`) and the repo/owner is `deepakpadamata/shoebox` (set in R1-13). Not verified: musl/cross builds of the iroh stack, the macOS lipo
  step, real minisign verification, the Debian/Raspberry Pi OS install runs. `scripts/test-install-keeper.sh` covers the script's control flow with a fake minisign (CI runs it).

### Checks that need real networks or hardware

Everything here was tested on loopback or in a browser on one machine.

- **A-025 · M0-01/M0-08** · The `hello-sync` spike ran on loopback with `MemoryLookup` only: relay, NAT traversal and
  real mDNS behaviour are *not* verified yet (M1-09 / M6-01).
- **A-042 · M1-08** · `LibraryNode` runs a sync manager: it syncs a known peer when it connects, when added, and when due
  (healthy peers every 5 min as a safety net behind iroh-docs' live gossip sync; failures back off 1 s → 60 s, doubling),
  and emits `SyncProgress {Started | Finished | Failed(reason) | Retrying{in_ms}}` per peer. `LibraryTicket` is a dev-only
  string (`shoebox-library1…`: capability + peer addresses + a checksum) that M2 pairing replaces.
  **Not verified here:** "two desktops on different networks, one via relay". Measured locally instead: 10,000 records written on
  one node converge on a second within 1.4 s of the last write (loopback; the writes themselves took 12.5 s).
- **A-044 · M1-09** · mDNS is on by default and off in `NodeConfig::local_only` (tests/explicit LAN-less use). `RelaySetting::Custom(url)`
  is parsed at open (a bad URL fails fast) but not contacted until used. `peer_info` classifies the in-use addresses
  (private/link-local/loopback → LAN, other IPs → Direct, else Relay) and reports the last ping RTT and last connection time.
  mDNS between two nodes on this machine is tested; **not verified:** with the internet physically unplugged, and the manual
  "relay → direct after hole punching" check (needs two real networks). Seen once (2026-10-26): `discovery::peer_info_reports_path_latency_and_last_seen`
  failed its "loopback counts as LAN" check while the Android emulator was shutting down its virtual network interfaces; 15 runs
  in a row passed afterwards. Not investigated further; watch for it in CI.
- **A-062 · M2-04** · Pairing as built: ticket = `{inviter addr, 32-byte secret, expiry, role}` (`shoebox-pair1` + hex + checksum, same shape as the library ticket; this string is what a QR
  encodes). The secret authenticates the exchange (keyed-BLAKE3 proof over both ids and a joiner nonce; the ticket is found by a key-derived 8-byte id); the six-digit code is a keyed hash of
  secret, ids and both nonces — people compare it, and *either* side declining aborts before anything is written. The inviter consumes the ticket on a valid proof (single use), expiry
  uses the node clock (default offered 5 min by the caller), and failed lookups are rate-limited per joining device (30/min, then that device is refused for a minute; a global limit let any stranger lock pairing for everyone, changed 2026-10-26; per-ticket proof
  failures burn a ticket after 5, though a ticket id cannot be guessed without the secret). The grant carries the device cert, the inviter's root-signed admin cert and the library
  capability; the joiner verifies the chain to the capability's root key before trusting it. The role is fixed by the ticket (a camera device scanning a QR gets what the QR offers).
  Admin rights are not granted by pairing; *Make admin* with the recovery phrase grants them (R1-04, `make_admin_with_phrase`; the others alert). **Verified: loopback only.** "Through a relay" is unverified (no relay available here; the protocol is plain QUIC streams so nothing
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
- **A-069 · M3 (architecture)** · The desktop app is `AppCore` (all logic, no UI toolkit) + two thin adapters: the Tauri shell and an HTTP bridge (feature `http`). Both call the same
  `commands::dispatch(name, json)`, so what the browser-based review and the end-to-end tests exercise is what the shell runs. The UI therefore talks to the core through **one** `command`
  invoke / `POST /api/<command>` and one event channel (`app-event` / `/events` SSE); `ui/src/lib/api.ts` is the typed client. Blobs go through `shoebox://localhost/blob/<photo id>?kind=…`
  (`http://shoebox.localhost/…` on Windows and Android). The dev bridge listens on loopback only and is not part of a shipped app. Verified in a browser pane against the real core: onboarding, kit,
  timeline with 9.5k photos, viewer, devices (QR + code check between two live instances), storage, settings, live updates over SSE. **Not verified:** anything that needs the actual Tauri window
  (rendering in WKWebView/WebView2/WebKitGTK, the `shoebox://` scheme in a real webview, tray, Keychain prompts, single-instance across launches).
- **A-070 · M3-01** · The app keeps its own `app.json` (device name, role, library ticket, relay) under the app data dir and does not read `shoebox-dev`'s `dev.json`; "opens an existing dev
  library created with `shoebox-dev`" is therefore not met (the formats differ and the dev tool keeps the root seed in the file). A one-off import command would be small but was not built.
  Single instance = an exclusive `app.lock` file in the data dir (second launch exits); the OS-level "focus the first window" is not wired. Closing the window hides it (tray keeps syncing).

### Security trade-offs

Accepted for now; for the external review and the beta.

- **A-045 · M1-10** · Presence is per library: topic = blake3(`shoebox/presence/2` ‖ identity id) (not the epoch or the root key, so presence
  survives a rotation and a root handover; the root key was dropped from it with A-088), so read-only and write devices share it and other libraries do not. Defaults: heartbeat 10 s, offline
  after 35 s (the issue's budget is 60 s). A `NewItems` nudge makes the receiver sync with the announcing device (only if it is a known library
  peer); imports announce automatically. `start_presence` must be called explicitly after `with_capability`.
  **Signed (2026-10-26).** Gossip relays messages through other peers and has been open to devices outside the device list since M5 (A-079),
  so each message is signed with the sender's device key over a domain tag, the topic and the body (device id, send time, message). A receiver
  drops it unless the signature matches the device it names, that device is listed and not revoked (a library with no device list yet
  accepts any signer, like the connection gate, A-061), and its send time is later than the last one accepted from that device (replays). The
  sender keeps its send times increasing even within one millisecond or when its clock steps back; after a restart with a clock that went back,
  receivers ignore it until its clock passes the last time they saw (at most the size of the step). Names over 64 characters and versions over
  32 are cut. Still open: anyone who knows the topic can *read* presence (device names, roles, app versions); the topic needs only the identity id
  (sent to anyone who dials `shoebox/recover/0`, A-068). The topic and message format changed with no N-1 overlap:
  nothing has been released. Tested: `presence.rs` unit tests (forgery, wrong topic, non-member, replay) and
  `failures.rs::presence_from_outside_the_device_list_is_ignored` (an outsider on the topic never appears; the test fails without the check).
- **A-057 · M2-01** · `LibraryState` verifies certs only when it knows the root (`with_root`; all three backends pass `cap.root`/`cap.library`). Local API calls
  (`apply_device_change`) reject bad signatures (`BadSignature`) and unauthorised signers (new `StoreError::NotAuthorized`). **Replicated** entries are kept
  and re-judged on every change, because a device cert can arrive before the admin cert that authorises it; only broken signatures are dropped. Consequence:
  anyone holding write access to the library space can park unauthorised-but-correctly-signed certs in memory (bounded by what they can write); they never
  affect membership. Without a root (unit tests only) the old "non-empty signature" stub applies.
- **A-061 · M2-03** · Enforcement is by **connection gating**: `Node::set_allowed_peers` (the non-revoked device list; empty set once this device itself is revoked) is applied in
  iroh's `before_connect`/`after_handshake` hooks, so it covers every protocol (docs, blobs, gossip, ping) and closes live connections of newly refused peers. Pairing
  (`shoebox/pair/0`) stays open to unlisted devices. Nothing is refused while no device list exists yet, so a fresh joiner can sync; a library whose first
  certs have not arrived is therefore unprotected for that moment. A revoked device that learns of its own revocation gets `Membership::Removed` +
  `NodeEvent::RemovedFromLibrary`; one that never hears (peers cut it off first) just cannot connect. **Not done / split into M2-03b:** capability rotation (the revoked device
  keeps the namespace secret), and per-record filtering of writes authored by a revoked device (unnecessary while it cannot connect, and not possible to enforce
  per record because registers merge by LWW without an author signature). The 100k-record rotation criterion moves with it.
- **A-067 · M2-03b** · "Rotation" on the Loro library means a new **epoch**: a fresh library id and a fresh Loro document, not a new secret (Loro has none). Certificates stay bound to the
  *identity id* (the original library id, carried in the capability's backend payload `[2, id]` after a rotation; `LibraryCapability::identity_id()`), so they remain valid without the offline root key.
  `revoke_and_rotate` (admin only) revokes, calls `LibraryStore::begin_rotation` (fresh document carrying every still-valid record, re-signed by the admin; other devices' holdings are not carried, each
  device republishes its own), signs a `RotationNotice` (revocation + admin cert + new capability) and pushes it over `shoebox/rotate/0` to every non-revoked member. Members verify the notice against the
  root key, apply the revocation it proves, call `adopt_rotation` (carry own records + the device list) and pull the rest from the others. Members keep the latest notice on disk and hand it to any device that
  connects, so a member that was offline catches up from whoever it meets first. The revoked device is never sent the notice and is refused at connection level (A-061); its stale epoch is unreachable.
  Limits: a device two rotations behind cannot chain notices (it rejects a notice whose `old` is not its epoch); holdings of other devices disappear until republished (the fetcher republishes on fetch only —
  a periodic republish is not implemented); apps must persist the new capability (`NodeEvent::LibraryRotated`: done for the keeper and `shoebox-dev run`); iroh-docs albums have a real bearer capability and are
  **not** rotated here (M5). Presence topics derive from the identity id so presence survives rotation. Measured: a 100k-record rotation takes 1.2 s in a release build and loses nothing (ignored test).
  **Found 2026-10-27 (while finishing R1-06):** a member that came back often got the same notice from two devices at once
  (one pushing it, another handing on the one it keeps), and both deliveries moved the store concurrently. The second
  "moved" from the new epoch into itself, building the replica in the directory it then deleted as the old one, which could
  drop records it had not carried (seen as a member missing a photo written before the rotation, about 1 run in 40 under
  load, with "No such file or directory" and a "bad signature" rejection in the logs). Fix: epoch changes on a node run one
  at a time (`epoch_lock`, each re-checking whether it already moved) and the Loro store refuses to move into the epoch it
  is in. Test: `rotation.rs::the_same_notice_twice_at_once_moves_a_member_once` (fails 10 of 10 without the fix); the
  rotation tests then passed 80 runs in parallel pairs.
- **A-078 · M5 trust model** · A shared album is an iroh-docs namespace; its capability is the key. A *viewer* gets the read capability, so viewers truly cannot write. *Owner* and *contributor* both receive the write capability, so on every replica they are told apart only by the membership entries, which any writer could forge: a hostile contributor could rename the album, add a fake owner or delete entries. The product assumes people you invite to contribute are not hostile (the same as handing someone a shared folder with write access). Removing a member (A-081) is the repair; a fake owner entry cannot be used to move the album away (members follow only the inviter they joined through, since 2026-10-26). Invitations are enforced at the inviter: each has its own random secret, used once and for its expiry (`shared-albums.json` keeps them); a joiner that was refused cannot fetch the album's content from the inviter, but the namespace capability inside the ticket is itself a bearer secret, so an interceptor who is *faster* than the intended person still wins. Expiry is also checked by the joiner.
- **A-079 · M5 connection gate** · A device with a library accepts only device-list peers (M2-03). Shared albums need strangers, so the gate lets three things through for anyone: pairing/recovery (as before), iroh-docs sync and gossip (guarded by the namespace capability, and the library itself no longer lives in docs), and the new `shoebox/album/0` handshake. Blob transfer (`/iroh-bytes`) is open only to *album peers*: members of an album this device is in, plus a joiner that presented a valid invitation in the last 30 minutes (until 2026-10-26 such a joiner kept blob access for good, even if it never became a member; now only members with an entry and recorded viewers count after the window, and the owner can remove a joiner that never wrote an entry). An album peer can fetch any blob whose hash it knows (hashes are unguessable and only appear in albums it can read), not just album blobs. When someone leaves the albums their open blob connections are closed. Docs sync also fetches entry content over the blob protocol, which is why a joiner needs the guest window.

- **A-085 · security review 2026-10-26** · **A stolen admin device could outlast its revocation; closed by rules that do not depend on
  dates.** A device cert is judged at its own HLC time (A-056) and the signer picks that HLC, so a stolen admin could backdate: write a
  revocation of the real admin dated before its own revocation (undoing it on every replica, through rotations too), or keep adding devices
  after it was removed. Now (`TrustChain::rules`): **only the root may revoke a device that holds an admin cert, or bring back one the root
  revoked**, and **once the root has revoked an admin device, none of the certs it signed count**, whatever date they claim. The "list of
  what stays valid" is carried as root re-signatures: in the same step the root re-signs the current cert of every device whose cert that
  admin signed (`revoke_device_with_root`, re-signed before the revocation so nobody drops out in between). That needs no new record type in
  the three stores. The root comes from the recovery phrase (`revoke_and_rotate_with_phrase`); if the device doing it is no admin, the root
  makes it one so it can sign the rotation. In the app, removing an admin device, or removing any device from a device that is no admin,
  asks for the phrase; an admin removing an ordinary device does not. A member that already followed the thief's rotation switches to the
  real one when that notice revokes, with the root key, the admin whose notice it followed (`accept_notice`); members that know an admin is
  revoked or expired refuse its notices. Costs and limits: removing a stolen *admin* device needs the recovery phrase (admins are the device
  that created the library and devices restored or acting with the phrase; pairing never makes one, A-062). Devices an admin had certified briefly have no valid cert on a member that took the thief's epoch, until it
  syncs the root's re-signed certs from the real admin. An admin whose admin cert *expired* can still backdate certs to before the expiry
  (no flow sets `expires_at` today). Data written before this change in which an admin revoked another admin now reads as not revoked
  (nothing has been released). Tested: `identity.rs::a_stolen_admin_cannot_backdate_its_way_back`, and
  `rotation.rs::a_stolen_admin_is_removed_with_the_recovery_phrase` end to end (fails without the epoch switch).

- **A-087 · detecting a leaked recovery phrase (2026-10-26)** · The phrase holds the root key, which cannot be rotated yet, so a leak
  must at least not go unnoticed. (1) *Approval:* a recovery through a device that has a screen waits up to two minutes for its person
  ("Let this device in?"); nothing is written before the answer, and no answer means no (`LibraryNode::set_recovery_approval`, the app
  always sets it; the headless Android worker cannot answer, so recoveries through it are refused). A keeper has no screen and lets
  recoveries through, logging a warning. The recovering device's screen says to approve on the other device. (2) *Alerts:* every device
  raises `NodeEvent::AdminAdded` for an admin it has not seen before; the app shows a red banner ("X became an admin of your library…
  remove it in Devices") until dismissed, persisted in `app.json`; the keeper logs it. A device starts alerting after its first finished
  sync (the creator at once), so admins that existed before it joined do not alert on it; an admin let in on this device does not alert
  here. Since R1-06 alerts also reach people with the app closed: Android's background worker posts each new one once on a
  "Security" notification channel, the desktop app shows a system notification when its window is hidden, and the keeper
  lists them in `status` until `shoebox-keeper alerts ack` (`alerts.json`). Each alert carries a `notified` mark, so a
  notification is shown once while the banner stays until dismissed. Checked on the Android emulator (an unnotified
  alert placed in `app.json`, the app closed, the worker run by hand): posted once on the "security" channel at high
  importance, not again on the next run. Not verified: alerts produced live on Android (the Rust tests cover that) and the
  desktop notification in a real window (R1-08). Not done: pairing a new non-admin device does
  not alert (it needs a ticket from inside). Root rotation followed (A-088). Tested:
  `recovery.rs::a_recovery_waits_for_approval_on_the_member`, `recovery.rs::other_members_alert_on_a_new_admin_but_not_on_old_ones`
  (fails if a device alerts before its first sync), `core.rs::a_recovery_through_this_app_asks_first`; checked by hand in three dev
  instances (approve, refuse, alert, dismiss).

- **A-088 · replacing the recovery phrase (root rotation, 2026-10-26)** · *Settings → Replace recovery phrase*, with the current phrase,
  on any member device (`LibraryNode::rotate_root`). A new root key is made and the library moves to a new epoch whose capability names it
  (`LibraryStore::begin_root_rotation`): every current admin and member is re-certified by the new root (the device doing it becomes an
  admin), photos, albums and settings are carried, the old certificates are not. So nothing the old root signs counts there, whatever
  date it claims, and the old phrase opens nothing (recovery, removal, role changes, another replacement). Members follow a `RootHandover`
  signed by the old root (only the current root may hand over) and the new one (whoever hands over holds it), delivered like a revocation
  notice (`EpochNotice`, also to devices that were away). Each member follows the first valid handover from the epoch it is in and refuses
  any other ("another epoch"): if someone else holds the old phrase it is a race, which the person wins on every device that hears from
  them first. Following devices raise `NodeEvent::RecoveryPhraseReplaced` (app banner "your recovery phrase was replaced on another
  device"); the new kit is shown once and has to be confirmed saved again. Limits: devices that were removed are not re-certified, so the
  removal itself is not carried (they are simply not members; pairing them again works as for any device); a device two epoch changes
  behind cannot follow (A-067); someone who had joined with the old phrase before the replacement stays a member unless removed first
  (the screen says so), and what they already downloaded cannot be taken back; the keeper CLI cannot start a replacement (it follows). The
  presence topic no longer includes the root key (A-045). Tested: `core rotation.rs::a_root_handover_needs_both_roots_and_the_current_one`,
  `rotation.rs::the_root_key_can_be_replaced_and_the_old_phrase_stops_working` (a non-admin replaces it, a member follows, one that was
  away catches up, photos stay, old phrase refused, new phrase works), `rotation.rs::the_first_valid_handover_wins` (fails without the
  epoch rule), `core.rs::the_recovery_phrase_can_be_replaced` (new kit once, saved flag reset, library reopens after a restart); checked by
  hand in two dev instances.

- **A-089 · how testers get updates (R1-07, 2026-10-27)** · Decided: a "new version available" notice, not Tauri's
  self-updater, for the preview. It works the same on desktop, Android (no store yet) and the keeper, needs no second
  signing key, and installs keep going through the signed downloads (R1-14). The app and the keeper read the site's
  `downloads/release.json` (`site/release.py`) at start and every 12 hours (`shoebox_node::update`, reqwest with the ring
  provider and bundled web roots already in the tree) and compare its version with their own; the app shows a banner with a
  link to the download page until dismissed for that version, the keeper mentions it in `status` and its log. A setting turns
  it off (Settings → About; `shoebox-keeper run --no-update-check`); the request carries only a user agent with the version.
  The feed is a placeholder until the site exists (R1-13) and the placeholder is never contacted; `SHOEBOX_UPDATE_FEED` (and
  `desktop-dev-server --update-feed`) point at another. The Android background worker does not check. The self-updater can
  come with the public release. Tested: `update.rs` (local feed; an ignored test reaches a real HTTPS site),
  `core.rs::a_newer_release_on_the_feed_shows_until_dismissed`, keeper `daemon.rs::status_mentions_a_newer_release`; banner
  and switch checked in a dev instance against a local feed.

### Product gaps and open decisions

Behaviour that will come back as features or needs a decision.

- **A-086 · roles can change (2026-10-26)** · A device's role lives in its signed device cert, so changing it is a new cert with the
  new role and the same name, signed by an admin (`set_device_role`) or by the root from the recovery phrase
  (`set_device_role_with_phrase`). A device cannot pick its own role: the role is access control (a viewer writes no library records) as
  well as storage policy. Every device follows the role in its own cert (`apply_membership`, on every device-list change and at start):
  it updates its node config (what the fetcher wants, whether it may free space), announces it on presence and emits
  `NodeEvent::RoleChanged`, which the app saves in `app.json`. An admin may change its own role and stays an admin. Camera-roll backup
  pauses on a viewer ("viewers do not add photos"). Replacement certs are dated after the cert they replace (`cert_hlc_after`): before,
  a device whose clock was behind the last signer's issued a cert that lost to the older one (this also affected removal and rename).
  Found while checking this in two dev instances: a device synced on its own initiative only with the peers it was given when it
  joined, so the device that started the library never pushed to devices that joined later, and a certificate change (role, name) waited
  for their 5-minute pull. Every listed, non-revoked device is now a sync peer and removed ones are dropped (`apply_membership`; test
  `revocation.rs::changes_reach_devices_known_only_from_the_device_list`, which fails without it). The app's sidebar also refreshes
  this device's name and role on device-list changes.
  Not done: the keeper CLI cannot change roles (use the app); the keeper daemon starts as Keeper and then follows its cert; nothing
  warns before demoting the only keeper (photos it held count as at risk again until another device holds them). Tested:
  `revocation.rs::roles_can_change_and_devices_follow_their_certificate`.

- **A-014 · M0-05** · `OrSet` keeps removed tags forever (grow-only). Compaction is deferred; at personal-album
  scale this is small. `Lww` equal-HLC/different-value ties (impossible in practice) resolve by value.
- **A-068 · A-060 closed** · Recovery without any capability: `Node::recover(link, phrase, role, name, now)` dials a surviving member on `shoebox/recover/0` (open to unlisted devices, like pairing). The member
  sends a challenge (nonce + the library's identity id), the device answers with a root-key signature over nonce/its id/identity plus an admin cert and a device cert for itself signed with the root key; the
  member checks the key is the library's root key and both certs chain to it, lists the device (as admin) and returns the current library capability. A wrong phrase changes nothing; failures are rate limited
  per device (10/min, then that device is refused; global until 2026-10-26, which let any stranger block recovery). The device is found through a `PeerLink` (`shoebox-peer1…`, no secret) that a surviving device shows (`LibraryNode::peer_link`, `shoebox-keeper address`). Not covered: finding a
  surviving device automatically (mDNS/relay lookup by library) and recovery when *no* device survives (the data is gone by design; the kit only restores identity). Recovering onto a rotated library works (the
  member answers with its current epoch). `shoebox-keeper recover` is the CLI entry.
- **A-083 · M5 not done** · Shared albums need a library on the device (no "shared albums only" mode, M5-02). No gossip presence, nudges or avatars (names are shared, M5-06). The accept-invitation screen has no storage-cost line. The UI loads an album page by page (500) but not virtualised. Notification posting when someone adds photos is not wired.

- **A-090 · adding photos on a phone (2026-10-27)** · The phone's Library has a ＋ button. *From the gallery*: with access to
  every photo it opens the system photo picker (`MediaStore.ACTION_PICK_IMAGES`) and adds the chosen photos in place, like
  the camera roll (`PhotoBackend.resolvePicked` maps a picker item to its MediaStore row and checks the file name, since
  the picker's ids are not a documented promise); with access to only some photos it asks Android to share more and adds
  what it then sees. *From files* opens the document picker: a gallery file found through it (`MediaStore.getMediaUri`)
  is added in place; anything else (a cloud drive, a USB stick) is copied, 20 at a time with the copies deleted after each
  batch, since the phone keeps no other copy of it. A gallery photo the app may not read in place is refused with a
  message, never copied. The photo picker returns at most `getPickImagesMaxLimit()` items per pick (100 on most phones).
  Verified with a fake photo source; the pickers need a person on the S25.
- **A-091 · storage per device (2026-10-27)** · *Devices → Storage* shows, per device, the originals it holds and their size
  plus its previews and thumbnails, from the replicated holdings (`LibraryNode::storage_by_device`): what that device last
  said it holds, not a disk measurement, and nothing outside Shoebox. On a phone the originals are its gallery files.
  Recently Deleted photos count until purged. It walks every photo's holders, so it is loaded after the device list rather
  than with it; fine for tens of thousands of photos, revisit with a per-device total in the store if it gets slow.
- **A-092 · the gallery owns its photos (decided 2026-10-27)** · Owner's rule: a phone never keeps two copies of a
  photo, and a deletion in the gallery is respected without a second deletion in the app. So:
  - Camera-roll originals are referenced where they are (`ImportMode::Reference`, iroh-blobs `TryReference`) and tagged
    `place/<hash>`; only previews and thumbnails are stored. iroh-blobs never deletes a referenced file (its delete path
    skips `DataLocation::External`). Files of 16 KiB or less are inlined by iroh-blobs (a copy, but negligible).
  - The scan reports each file's path (MediaStore `DATA`) when it is readable in place; a photo without one is not added
    (reported, retried), never copied. Read by path, a photo keeps its GPS when `ACCESS_MEDIA_LOCATION` is granted.
  - `backup.json` tracks asset id → photo, path and modified time. A tracked file that is gone while its storage
    (`/storage/emulated/<n>`, `/storage/<card>`) is still there was deleted: the photo is deleted from the library
    (Recently Deleted for 30 days on every device, then purged everywhere, keeper copies included), and this phone's
    holding is retracted at once. Same content still in the gallery under another file keeps the photo. An edit (same
    asset, new content) adds the new version and deletes the old one. A card that is not mounted is not a deletion.
  - With only some photos shared, Android hides every other file (the S25 showed the app 22 of 3,114 files in the
    camera folder), so a deleted file and an unshared one look the same. A shared file stops being visible when it is
    deleted or unshared, and unsharing happens when access changes. So: files that vanish across Shoebox's own "share
    more" prompt, or when access narrows from all photos to some, were unshared (kept in the library, no longer held
    here); any other disappearance is a deletion. Not caught: unsharing single photos in the system settings while
    access stays "some photos" counts as a deletion (Recently Deleted, restorable for 30 days). Without access nothing
    is checked. (First build checked only with full access, and a deletion on the S25 was missed: found 2026-10-27.)
  - An original that cannot be read when the viewer asks for it (the file changed or vanished) triggers a gallery
    check; if the photo is still referenced after that, this phone stops holding it and reports it as not here, so the
    viewer shows the preview and fetches the original from another device. A released blob counts as not here
    (`BlobStore::kept`) until collection removes it; fetching it waits for that (up to 90 s, normally under 30 s).
  - More than 50 gone at once and more than a quarter of the tracked photos waits for the person (banner: delete them
    too, or keep them in the library; kept ones are no longer held by this phone).
  - Free-up never counts `place/` originals: the file is the gallery's. Free-up on a phone only frees originals it
    fetched or copied in.
  - Upgrading: a phone whose `backup.json` predates this looks at the whole roll again; each photo comes back as a
    duplicate, and a stored copy is released and re-referenced once the store's collection has removed it (`moving`;
    up to one GC interval: the app collects every 20 s once idle for 10 s, a phone's battery cost is a tag listing per pass).
  - Desktop imports still copy (a folder on a computer is not a gallery that deletes on its own); a by-reference import
    for folders the person promises not to change remains possible (A-028).
  - Checked on the S25 (Android 16, access to selected photos only, 2026-10-27): the selected files are readable by path,
    all 22 were tracked in place, and the 22 copies an earlier build made were replaced within one GC interval (the
    app's blob data went from 190 MB to 17 MB); a gallery deletion with some photos shared reached the library on the
    next check. Not checked: GPS kept through the path (none of those photos had a location), the photo picker with
    full access.
  - Not handled: a shared album that references a deleted gallery photo keeps a dangling reference on this phone until
    the photo is purged (the album's other members have their own copies). iOS (PhotoKit) is not built.
  - Photo-library assets (P1-02, 2026-11-03). PhotoKit has no file paths, so an asset's original is either a file
    (Android) or a library item read through the source by its id (`Original::Library`). Library assets are tracked
    by id (`backup.json` `tracked` entries without a `path`). One is deleted when the scan reports it removed; that
    removal is saved in `removed_from_library` before the token moves. With backup off, the gallery check scans when
    library assets are tracked. Unsharing works as above: assets hidden across Shoebox's prompt, or when access narrows
    (found by a full scan), stay in the library. They are not in any backup scope's folders, so every scope takes
    them. An original only in the cloud is reported and waits. Until P1-07 serves them on request, library
    originals are read into a temporary file and copied in. Not built for a real platform yet (P1-03/04/06); tested
    with the fake source only. A `backup.json` from before loads unchanged.

- **A-093 · the Companion role and its keep rules (2026-10-27)** · "Phone" became **Companion** (`Role::Companion`; the serde
  alias `Phone`, the app's `parse_role` and the dev tool still accept the old name, so config files and app.json from
  development builds load; certs are postcard, which stores the variant index, so devices on either name still agree).
  A Companion fetches thumbnails and previews of everything and the original of a photo when it is pinned, recent
  (taken within `keep_recent_days`, default 90, 0 = off), a favourite while `keep_favorites` is on (default), or its own
  capture below the copy target (`PlanContext::keeps_original`). The rules are device-local in `config.toml`
  (`keep_recent_days`, `keep_favorites`), not replicated: each device chooses for itself. Free-up never frees a pin, a
  recent photo or a kept favourite. Nothing is removed automatically when a photo leaves the window or a toggle goes
  off: the originals stay until *Free up space* (an automatic offload is a possible follow-up). "Recent" is the photo's
  taken date, so a photo shot long ago and imported today is not recent; a favourited photo is fetched on every device that keeps
  favourites, which on a small phone can add up (the toggle is there for that). A phone's gallery photos stay in
  place whatever the rules say (A-092). Onboarding on a phone or tablet defaults to Companion.
- **A-094 · pins were forgotten on restart (found and fixed 2026-10-27)** · `Node::open` rebuilt `config.toml` from the caller's
  defaults and kept only watched folders, so `pinned_originals` (the list the fetcher and free-up read) reset to empty
  every start while the `pin/` blob tags stayed: after a restart a pinned original was no longer planned and free-up
  could release it. It now keeps pins and the keep rules too (`pins_survive_a_restart`).

- **A-095 · the keeper's CLI as an admin (2026-10-27)** · `shoebox-keeper devices remove|role|make-admin|replace-phrase` call the
  same node operations as the app (`revoke_and_rotate[_with_phrase]`, `set_device_role[_with_phrase]`,
  `make_admin_with_phrase`, `rotate_root`) over the control socket. The recovery phrase travels over the 0600 Unix
  socket in the request and is never logged; the terminal prompt hides the typing with `stty -echo` (or reads a line
  when stdin is piped). Devices are named by full id, unique id prefix (≥4 hex digits) or name. Until now `devices
  revoke` only marked the device revoked and did not move the library to a new epoch (the app did): fixed, so a
  device removed from the keeper is locked out of what is written next like one removed in the app. `devices add` offers a pairing string (no QR: the person pastes it into the new device's
  app, which already accepts a pasted ticket) and relays the six-digit code over the same socket conversation as `pair`;
  it needs the keeper to be an admin like the app, and says how to become one.
- **A-096 · devices shown offline while connected (found on real devices, fixed 2026-10-27)** · With the Mac, the
  keeper on nest and the S25 on mobile data, every device showed the others offline although sync ran over direct
  paths. Cause, in iroh-gossip 0.101: a peer whose first dial fails stays `Pending` in the gossip actor and every later
  message to it is queued without a new dial, so one failed join (a phone asleep or on another network, a keeper
  restarting) silences presence and "new items" nudges with that device until the app restarts. Fix: every third
  heartbeat, presence opens a gossip connection itself to each listed peer it does not hear from
  (`Node::gossip_dial`, which hands the connection to `Gossip::handle_connection` and so flushes the queue) and joins it
  again. Test: `presence_reaches_a_peer_that_was_unreachable_at_first`. After deploying, the S25 on LTE showed online
  and direct on the keeper within seconds. Shared albums' iroh-docs live sync uses the same gossip; their sync paths
  already re-ask silent peers (R1-02). Also from the same run: the keeper listed devices by their certificate names
  ("This computer", the onboarding default when the name field was left empty) while the app showed the names devices
  report; the keeper now uses the app's rule, and onboarding on a phone says "phone" and defaults to "My phone".
- **A-097 · shared albums belong to the person, not the device (owner's request, 2026-10-27)** · Three asks after
  trying shared albums on real devices: share an album you already have, show one name for a person, and give all of
  a person's devices access when one joins. Design:
  - *The library holds the person's shared albums.* Library setting `shared.albums` is a JSON list of `AlbumRef`
    (capability, inviter, role, `cap_ms`, `joined_ms`, `left_ms`, `from_album`, `location`). Every device (the app
    and the keeper) follows it (`shoebox_store::library_link::follow`, on library changes and every 20 s): albums it
    is in are added; albums in the list it is not in are *adopted*: the namespace is imported with the capability,
    the album's record fetched from the person's other devices (then the inviter), and the device takes part with
    the same role, with no "joined" line in the feed. Album capabilities were already in each device's
    `shared-albums.json`; now every device of the library can read them too (same trust domain: the person).
  - *Leaving* writes `left_ms` and every device leaves. *A move* (the owner removed someone) gives a new capability;
    `cap_ms` decides which is newer, so a device that was not told directly (a viewer device without an entry)
    follows through the library. A list write is last-writer-wins: two devices joining different albums at the same
    moment can drop one entry, which the device that is in it re-adds on its next round.
  - *One person, several devices.* A new album record `people/<device>` says which person (the library's lasting
    identity id) a member device belongs to. Members are listed per person with a device count, the 50-member limit
    counts people, and removing a member removes all their devices. Older builds skip the record (they show each
    device as before).
  - *The person's name* is the library setting `profile.name`, set in Settings → You; until set, a device's own name
    is used, as before. Changing it renames the person in every album.
  - *Sharing an album you have* (Albums → the album → Share this album) creates a shared album with its name and
    photos, asks once about location (remove or keep, for later photos too; with none to ask about, later photos have
    their location removed) and links the two (`from_album`): photos added to the album later are shared by whichever
    of the person's devices is in the shared album (at once on the device that added them, otherwise within 30 s).
    Removing a photo from the album does not remove it from the shared album (shared albums cannot remove items yet).
  - Limits: a *viewer* person's other devices have no member entry, so other people's devices do not serve them
    blobs; they get the album's photos from the person's own devices. Tested in
    `shared_albums_follow_the_person_to_all_their_devices` (two devices of Ana, Ben; name, adoption, one person with
    two devices, one "joined", leaving everywhere, sharing an existing album and a later photo following).

- **A-098 · Android plays videos over loopback HTTP (owner's report, 2026-10-27)** · A 140 MB HEVC video taken on the
  S25 never played on the phone ("Get original" looped: the original was there, the `<video>` silently failed). Three
  causes, found with the WebView debugger on the phone:
  - *Android's WebView re-applies byte ranges.* Chromium's stream loader for `shouldInterceptRequest` responses
    (`components/embedder_support/android/util/input_stream_reader.cc`, `SkipToRequestedRange`) skips the request's
    first byte position in whatever stream the handler returns, so a correct partial answer from `shoebox://` is cut
    again and fails; only requests from byte 0 worked, and a long video needs many pieces. Video originals on Android
    are therefore served by a small HTTP server in the app (`src-tauri/src/media.rs`): bound to 127.0.0.1, every path
    starting with a random 256-bit token made at each start (other apps on the phone can reach the port but cannot
    name a blob), `GET`/`HEAD` only, one request per connection, `no-store`. The CSP allows `http://127.0.0.1:*` for
    `media-src` only; the release network security config allows plain HTTP to 127.0.0.1 only (debug: anywhere, for
    `tauri android dev`). Photos still use `shoebox://`; desktop is unchanged.
    A stopgap: R1-16 replaces it with Android's own player and `content://` URIs for media already on the phone.
    **Replaced (R1-16, 2026-11-05):** the server, the `media_base` command, the 127.0.0.1 CSP entry and the network
    security config are gone; the app opens no port. Videos play in a Media3 player over the app (`PlayerActivity`):
    a gallery video from its file where it is, any other from the blob store through `BlobDataSource`, which reads
    1 MB pieces from the running app core over JNI (`NativeBridge.readBlob`, `video.rs`), checked against the hash.
    Posters of videos this phone holds come from the system's frame grabber the same way (`make_poster`), not from
    the WebView. A Live Photo's short clip is read whole into a `blob:` URL and still plays inline. Gallery *photos*
    still load over `shoebox://`: they are small and unranged, which the WebView handles; whether it can load their
    `content://` URIs directly was not checked. Checked on an emulator (Android 16): a 90 s gallery video and a 60 s
    store-only one start at once, seek, close with Back, get native posters, and no app socket listens. **Not checked
    on the S25:** the 140 MB HEVC video, and a video fetched from the keeper and then played.
  - *Open-ended ranges got the whole file.* `bytes=N-` (what a media element sends) now gets at most 4 MB.
  - *Range reads were slow.* `BlobStore::open_range` read through iroh-blobs' seek-and-read reader: 1.4 s per 4 MB on
    the phone; one `export_ranges` request takes 70 ms (60 MB/s), well above a phone video's bitrate.
  - Verified on the S25 (debug build, via the WebView debugger): metadata in 64 ms, 6 s of playback in 6 s, a seek to
    70 % and 5 s more, no stalls. The release network security config has not run (needs a release build, R1-14).
  - The viewer now says when an original that is on the device cannot be played instead of offering "Get original"
    again.
  - *What the owner actually saw* was a fourth cause, in the viewer, on the Mac and the phone alike: a video's
    `preview` and `thumb` are the original (videos have no still renditions yet, M3-10), the viewer showed the preview
    in an `<img>` while waiting, and when that image failed (after the `<video>` had replaced it and started) it
    reported the original unreadable and removed the video. Videos now show only the device's poster frame.

- **A-099 · a blob comes from several holders at once (owner's request, 2026-10-27)** · Fetches tried holders one at
  a time, each from the start of what was missing. iroh-blobs' downloader does not split one blob across providers:
  its `Split` strategy only splits the children of a hash sequence; for one blob it fails over provider by provider
  (resuming). So `Node::download_blob_from(holders, hash, size)` does the split itself on iroh-blobs' range API:
  - A blob of at least 8 MiB (`SPLIT_MIN`) with more than one holder is cut into 2 MiB pieces; up to four holders
    (`MAX_SOURCES`) each connect and take the next piece (`remote().execute_get` of that chunk range), so a faster
    holder takes more. A piece a holder fails on, or answers only in part, goes back for another and that holder stops.
    The split ends as soon as no piece is waiting or in flight, without waiting for a holder still trying to connect.
    Every piece is verified against the BLAKE3 hash as it arrives, whichever holder sent it.
  - Whatever is still missing, and every smaller blob, goes through the downloader with the whole holder list: one
    holder at a time, each continuing where the last stopped.
  - The library fetcher (and so *Get original*) passes the ranked holders and the size from the record. Shared albums
    pass their members without a size: members are not known to hold a photo, so they fail over but do not split.
  - Measured with `cargo run --example fetch_bench` (`crates/shoebox-node/examples`), 128 MiB fetched on the S25
    (debug build, home Wi-Fi) from the MacBook Air and nest: Mac alone 6.5 s (20.6 MB/s), nest alone 12.1 s
    (11.1 MB/s), both 4.75–5.5 s (25–28 MB/s; the Mac sent about two thirds). Not yet measured on mobile data
    through a relay. The first fetch from a holder that had just started failed in 1 s (its address was not
    published yet); the app's holders are long-running, so this should be rare.

### Rules for future changes

Not open questions, but they must be honoured by any format or protocol change, and re-checked on every iroh upgrade (shared albums still run on iroh-docs).

- **A-008 · M0-04** · Record encoding is `[version byte] ++ postcard(body)` with the version defined per type
  via `Record::VERSION` (all `1`). There is no type-tag byte; the *key/namespace* identifies the record type.
- **A-096 rule** · On an iroh-gossip upgrade, check whether a failed dial still leaves the peer `Pending`
  (`net.rs`, `OutEvent::SendMessage`); if fixed upstream, `Node::gossip_dial` can go (the presence test must still pass).
- **A-036 · M1-05** · iroh-docs 0.101 on iroh 1.4 reports `Failed to close connection` from the *closing* phase of an
  otherwise successful sync; `sync_with` treats exactly that error as success (any other `SyncFinished` error fails).
  Also required: `start_sync(vec![])` on every opened namespace, otherwise incoming syncs abort with `NotFound`.
- **A-043 · M1-05/08** · Lesson recorded: iroh-docs delivers events through bounded channels and a subscriber that stops draining
  stalls the whole engine (including the queries a rescan needs). The docs backend therefore drains events in a dedicated task
  that only signals "dirty" and rescans in another. `sync_with` also keeps rescanning (max 10 s) until no entry is waiting on its
  content, because `PendingContentReady` does not guarantee all content is local (this caused a rare flake in the 3-node test).
- **A-047 · M1-05/08/11** · Hardening of the docs backend found by stress runs (each run of the docs test binaries, 25–40×, before/after):
  (1) `SyncFinished` carries no reliable outcome in iroh-docs 0.101/iroh 1.4: nearly every sync reports "Failed to close connection",
  including complete ones and ones where two syncs between the same peers overlapped and *no data moved*. So `sync_with` now asks the peer for
  an order-independent **digest** of its namespace (`shoebox/doc-digest/0`, a tiny protocol served by every node) and syncs again until the
  digests match (≤ 6 rounds, then it accepts — live writes can keep digests moving). It costs one O(entries) listing per round on each side.
  (2) A sync request is ignored while another sync with that peer is running, and that one may predate the data: only a sync that *started
  after the request* counts, and a stale finish triggers a fresh request. (3) iroh-docs fetches an entry's content once per sync run and
  never retries, so the replica fetches content that has been missing for 750 ms itself, from peers it has exchanged entries with, retrying
  every 4 s and rescanning every 500 ms while anything is pending. Flake rate went from ~15% to 0 in 90 consecutive runs.
- **A-051 · M1-13** · Found while adding fast blob GC to the testkit: iroh-docs + iroh-blobs GC can delete a just-written entry's content on the
  *writer* (raw iroh-docs, no networking, GC every 100 ms: 10 of 3000 entries lost their blob). The replica now writes content itself
  (`add_bytes` + `set_hash`) and holds a temp tag for 60 s after the entry exists. The testkit clusters run GC every 200 ms, so every multi-node
  test doubles as a regression test for this. Production GC defaults to every 5 min, which makes the race rarer but not impossible without the hold.
- **A-052 · M1-13** · Blob GC is gated on node *activity*: the node records when it last imported, fetched, wrote blobs/docs entries or synced, and the
  GC protect callback aborts a pass if that was within `gc_quiet_ms` (default 30 s; testkit 400 ms). iroh's snapshot-then-sweep GC can otherwise
  delete blobs created or referenced mid-pass (reproduced as lost entry content and lost library records under a 200 ms GC). Also verified
  experimentally: a *temp* tag keeps a **partial** blob alive through GC while held (a named tag does not), so a failed fetch now leaves a temp tag on
  the partial blob for up to an hour to preserve progress for the retry (in-process only: after a restart GC discards partials).
- **A-084 · M6 decisions** · (1) *Format marker:* `format.json` in the app data dir and the keeper storage dir (layout 0 = data without a marker, adopted with a backup; 1 = the marker; 2 = 0.1.0, the baseline from which compatibility counts, R1-01: the epoch notice moved to `epoch-notice.bin`). Every versioned name and file is listed in `docs/notes/formats.md`, kept in step with the source by `tests/formats.rs`. The backup copies files up to 64 MB outside `blobs/`, `posters/` and `backups/`, so it holds the small state (app.json, config, registries, Loro snapshot/log up to that size) but not the blob store or a very large docs database; future migrations that rewrite those must carry their own safeguard. (2) *N and N-1:* protocols are named with a version (`shoebox/loro/0`); a release that changes one serves old and new for a release. Nothing has needed a second version yet, so the rule is tested (names carry versions, an unserved version is refused cleanly) but not yet exercised by a real change. (3) *Diagnostics:* the log tail is an in-memory ring (400 lines, INFO and above) installed by `init_logging`; redaction is by token (paths, file names, coordinates with four or more decimals, e-mail addresses, invitations, long hex) plus the names this person gave things, and errs on the side of removing too much (a plain decimal like `1.2345` becomes `<coordinate>`). The report is shown for copying or saving; the OS share sheet is not wired. (4) *Accessibility:* contrast was checked against WCAG AA (4.5:1) for the design tokens and the failing ones were darkened (ink-3, accent, ok, warn in light; ink-3 in dark), so the app is slightly darker than the mockups; *Text size* uses CSS `zoom` on the page because several webviews ignore the system font size; screen-reader behaviour is untested. (5) *Failure scenarios:* two libraries that merely share a capability, without certified devices, do not exchange records (found while writing the clock-skew test); last-writer-wins follows the hybrid clock, so a clock a day ahead wins conflicts until the others catch up.

## Settled

- **A-003 · M0-02** · Rust edition 2021 and `rust-version = "1.85"` (conservative MSRV); the toolchain itself is `stable`.
  Why: iroh 1.x needs a recent compiler; 2021 avoids edition-2024 churn in macro-heavy deps. Reverse: bump edition/MSRV.
- **A-004 · M0-02** · License files: `LICENSE-APACHE`/`LICENSE-MIT` at the root (copyright "Shoebox contributors",
  text copied from the iroh-docs crate's standard licenses), and `apps/LICENSE-AGPL` (official GNU text).
  `tools/*`, `xtask` and `examples/*` are treated as library-licensed (Apache-2.0 OR MIT), since decision #17
  only names `apps/*` as AGPL. Reverse: change the `license` field in the affected crates.
- **A-005 · M0-02** · `apps/desktop-mobile` is a placeholder `fn main() {}` crate now; the Tauri project replaces it in M3-01.
- **A-007 · M0-03** · `Unlicense` added to the allow-list (public-domain-style, pulled in by wasm-only transitive
  crates `async_io_stream`, `pharos`, `ws_stream_wasm`). `unused-allowed-license = "allow"` so the list can be a
  superset. Reverse: remove from `deny.toml`.
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
- **A-026 · M0-08** · `jpeg-encoder` (xtask only) is `(MIT OR Apache-2.0) AND IJG`; IJG is permissive but needs
  attribution in shipped binaries, so `deny.toml` carries a per-crate exception instead of allowing IJG globally.
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
- **A-037 · infra** · The dev profile uses `debug = "line-tables-only"` (none for dependencies). Full debuginfo for the iroh stack
  grew `target/` to 13 GB and filled this machine's disk mid-session; the lean profile keeps it ≈2 GB. Reverse in the root `Cargo.toml`
  if you need full debuginfo for a dependency.
- **A-038 · M1 ordering** · The dev CLI (M1-07) is implemented *after* M1-08…M1-13: its `join`, `peers`, `holdings` and `fetch`
  commands and the demo script need sync, discovery, presence, holdings and the fetcher. Everything else follows the plan order.
- **A-039 · M1-06** · The node-level orchestration lives in `shoebox_node::LibraryNode<L: LibraryStore>`, generic over the backend
  (the docs backend depends on `shoebox-node`, so `Node` itself cannot own a store). Tests use the mem backend for speed.
- **A-041 · M1-06** · Watched folders are debounced 400 ms after the last event, persisted in `config.toml` (kept across restarts even
  when the caller passes a fresh `NodeConfig`), and rescanned on start so files that arrived while closed are imported.
- **A-046 · M1-11** · Added a replicated, last-writer-wins `settings` facility to `LibraryStore` (`setting`/`put_setting`, keys `[a-z0-9._-]{1,64}`,
  opaque byte values) with its own conformance scenario; the redundancy target is the setting `redundancy.target` (one byte, default 2, ≥1).
  Copy counts include devices that are currently offline (their data exists). `at_risk` over 100k photos takes ~22 ms in a release build
  (the debug-build test uses 10k photos because *building* a 100k library in debug takes over a minute).
- **A-048 · M1-12** · The fetch policy is `shoebox_core::policy` (pure): Keeper = all; Phone = thumbs+previews, plus an original only when pinned or
  it is this phone's own capture with fewer copies than the redundancy target; Viewer = thumbs+previews. Shared-album items: previews, and the
  original (or the location-stripped `shared_hash` copy) only for Keepers with "keep originals". Videos use the original as thumb/preview until
  M3-10, so a viewer *does* have to be able to open them on demand, but never pulls them in the background (the placeholder is only ever
  planned as an original). Queue order: all thumbs → all previews → all originals, newest first. Holder choice: best network path (LAN >
  direct > relay > unknown), then most recently seen, then id; failures fall through to the next holder, and a failed blob cools down for `retry`.
- **A-050 · M1-13** · Blob reclamation is *reconciliation*, not bookkeeping at purge time: a purge removes the record (on every device), so each device
  independently releases its `lib/`+`pin/` protection on blobs that no live or soft-deleted record references and retracts the holdings; iroh-blobs'
  periodic GC then deletes whatever has no protection left. This works however the record vanished (local purge, replicated purge marker, crash in
  between) and leaves `album/<id>/<hash>` protections alone (the M5 shared-album hook; verified: an album-referenced preview survives the purge).
  An `ingest` RwLock keeps a reconcile from releasing a blob whose record is not written yet. `start_maintenance` runs purge+reconcile hourly and
  within a debounce (default 5 s) after any delete/purge change, on every device. A purge is performed by whichever device's clock passes the 30-day
  mark first; markers are monotonic so concurrent purges agree.
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
- **A-056 · M2-01** · Identity = ed25519 (dalek 3, strict verification). Signatures cover a domain-separated, library-bound message
  (`shoebox/{admin,device}-cert/v1` + library id + postcard of the cert fields), so a cert cannot be replayed into another library. A device cert signed by the
  root has `added_by` = the root public key bytes. Authority is judged **at the cert's own HLC time**: the signer needs a root-signed admin cert that had
  started and not expired by then and must not have been revoked earlier; the chain is replayed in HLC order, so the result never depends on arrival order.
  `AdminCert` gained `expires_at: Option<Timestamp>` (golden bytes updated). A revoked admin's earlier certs stay valid; later ones do not.
- **A-058 · M2-01** · `shoebox-dev init` creates a random root key, self-issues an admin cert and a device cert, and keeps the root seed in `dev.json`
  (dev tool only; real devices never store it, see M2-02). `shoebox-dev join` does not yet obtain a cert: pairing (M2-04) does that.
- **A-059 · M2-01 · closed by R1-02** · Two flaky tests, both fixed on 2026-10-26 and run 50 times in a row since.
  (1) `shoebox-store-docs` `autosync::sync_now_waits_for_a_sync_already_in_flight` (about 1 in 6): a real stall, not a test
  problem. The library's (and an album's) `sync_with` held an iroh-docs event subscription and stopped reading it between
  sync rounds while comparing digests; 300 records' worth of events filled the bounded queue and iroh-docs blocked, including
  the next `start_sync`, until the 10 s timeout. The same lesson as A-043, in the sync path itself. Fix: the sync paths read
  their subscription through `album::drained` (an unbounded queue fed by a task); `sync_round` also asks again every 2 s
  when iroh-docs answers a request with no event at all (up to 8 times). Confirmed: without the draining it fails again
  within 5 runs. (2) `freeup::dry_run_and_real_run_agree_and_pins_are_kept` and `too_few_other_copies_means_nothing_is_freed`
  (2 in 10, in both the docs and the Loro copies of the test): a race in the test setup, which waited until the phone knew the
  keepers' copies of the *first* photo only; a dry run before the other photos' holdings arrived rightly freed less. Free-up
  itself was safe throughout (it frees less, never more). Fix: the setup waits for every photo.
- **A-060 · M2-02** · The kit holds the *root key* (24 words = its 32-byte seed as BIP39 entropy, English list; typed phrases are case/space-forgiving) plus library id and
  creation date in a printable text file (`RecoveryKit::to_text/from_text`). `LibraryNode::create_identity(root, …)` returns the kit and keeps nothing (the issue's
  `Node::export_recovery_kit()` cannot exist because the root is never stored; export happens once, at creation). `restore_from_kit(phrase)` verifies the phrase against the
  library's root key before writing anything, then certifies the new device as admin member. **Gap (acceptance "rejoins and sees all photos once a keeper is reachable")**:
  the kit does not contain the library's *space capability*, so a device that lost everything cannot yet fetch data; this needs a root-proof handshake in which a surviving
  member/keeper hands the capability to a device that signs a challenge with the root key — scheduled with the pairing protocol (M2-04). `recovery_kit_saved` is the
  replicated setting `recovery_kit_saved` (`[1]`).
- **A-064 · M2-08** · Free-up's "≥ target copies elsewhere" is implemented literally: other devices' *originals* (replicated holdings, excluding
  this device) must number at least the redundancy target (default 2) **and** at least one of those holders answered a `shoebox/has/0` probe for
  that blob within 7 days (confirmations persist in `holder-confirmations.json`; a holder that denies a hash is forgotten for it). Free-up probes
  holders itself, in dry runs too, so both runs agree. Pinned originals, blobs a shared album still references, and keepers (error) are excluded.
  A real run releases the library/pin protection and retracts holdings; the bytes disappear when the blob store's GC runs (reported bytes = sizes
  of the blobs released). A phone does not re-fetch freed originals only because its fetcher policy wants originals of pinned or own below-target
  photos, and freed photos have enough other copies; revisit if a pin/own policy ever overrides that.
  **Brought in line with "backed up" by R1-03 (2026-10-26):** an original is also freeable when a certified, non-revoked
  keeper holds it and that keeper itself answered the `has` probe within 7 days, whatever the target (`freeup.rs`). Before,
  a photo on a phone and one keeper showed as backed up but could not be freed. Tested: Loro
  `freeup.rs::one_confirmed_keeper_copy_is_enough` (fails without the rule); the "too few copies" test now uses a holder
  that is no keeper. The docs-backend tests have no certificates, so no keeper, and test only the copy-count rule.
- **A-071 · M3-03** · TypeScript bindings come from `ts-rs` (`#[ts(export)]` on the shared types, written to `ui/src/bindings` by `cargo test`, `git diff --exit-code` in CI) instead of `tauri-specta`:
  the command surface is one generic `command(name, args)` shared with the HTTP bridge, which specta's per-command generation does not fit. Argument shapes are checked at run time (`commands.rs`
  `arg`/`opt`), results are typed on the TS side. A mismatch in a *result* type fails the build; a mismatch in an *argument* only fails when called. 64-bit numbers cross as JSON numbers
  (`f64` in the bindings): fine for millisecond timestamps and sizes, not for ids (those are hex strings).
- **A-081 · M5-05 removing a member** · The owner writes a new namespace with the same album id, items, activity and the members that stay, and pushes `Moved{capability, owner}` to each of them over `shoebox/album/0` until acknowledged (retried every few seconds while the owner runs). A member accepts only if the sender is the inviter it joined through (only the owner invites); until 2026-10-26 it checked the membership entries, which any contributor can forge, so a contributor could make itself "owner" and move everyone, the real owner included, to a space it controlled. The owner's own device has no inviter, so it never follows a `Moved`. Contributors get the write capability, viewers the read capability. Viewers cannot write a membership entry, so the owner records them when they present their invitation (`viewers` in the registry; they are shown as "Viewer ab12cd34"). A member that is offline when the owner is online follows when both are next online; until then it keeps working in the old space, where nobody who stayed will see its additions. The removed member keeps the photos they downloaded, still has the old namespace (frozen), and is dropped from the owner's blob gate at once. The owner cannot be removed and leaving as owner leaves an album nobody can invite to.
- **A-082 · M5 limits and scale** · 50 members and 50,000 photos are enforced when inviting and adding (clear messages). Writing 50,000 items takes about a minute on a laptop (`cargo test -p shoebox-store-docs --test album_reopen -- --ignored`); the "50k items to a new member in under 2 minutes" criterion is covered by `album_sync_of_50k_items` (ignored, run on demand).

- **A-107 · search models measured (S1-01, S1-08…S1-12, 2026-10-30)** · On the Mac, OpenCLIP ViT-B/32 at 8 bits finds the
  captioned photo of 1,000 COCO photos (indexed from 256 px thumbnails) at R@1 60.5 / R@10 93.7, as well as at full
  precision; storing vectors as `vec8`, or `vec1` first, costs under half a point. The text model takes 14 ms a query
  and about 510 MB resident while loaded (it is let go after two idle minutes); scoring 100k photos takes 8 ms.
  **Android uses tract too**, not ONNX Runtime through Kotlin: one code path; NNAPI speed is unmeasured. Not measured:
  the S25, a Pi 5, nest. The label list has 937 labels (the plan said 2,000–5,000): role and composite labels
  misfired on the eval set and were dropped; captions confirm 56% of kept labels (a floor: captions leave things out).
  Text in photos is read on macOS (Apple Vision) only; Android ML Kit, Windows OCR and a keeper engine are not written.
- **A-108 · people and model upgrades (S1-15, S1-18, 2026-10-30)** · A Viewer publishes no people records (S2). Old
  models are retired without a record: a device lets another model's shards go once its own model's `vec8` shards
  cover at least as many photos (S3), so a device still on the old build keeps its own copies. Naming a group labels
  its five best faces; the group's name follows the majority of its labelled faces. Grouping is greedy leader
  clustering (cosine ≥ 0.45), not constrained agglomerative clustering; "not Ana" is enforced at resolution.
  Desktop indexing on Linux and Windows assumes mains power and an idle user when it cannot tell (macOS asks
  `pmset` and `ioreg`).

- **A-109 · F2.10** · **Live Photos and motion photos.** A photo has at most one video, in a `motion/<photo>` record (`MotionRecord`) that points at a blob of its own; the video is never an item of the timeline. *Pairs:* a `.MOV` or `.MP4` of at most 10 s next to a `.HEIC`, `.HEIF`, `.JPG` or `.JPEG` of the same name in the same folder (case aside). Decided from the folder, not the batch, so a watched folder that hands the two over separately still pairs them; both files are always imported together. Not checked against Apple's content identifier (in the MOV's metadata and the HEIC's MakerNote), so a 10-second clip that happens to share a name with a picture is taken for its video; a longer one is a video of its own. A picture that cannot be read leaves its clip a video like any other. *Motion photos* (Samsung, Google): the picture stays the whole file; the video is found by structure, as the first complete MP4 after the first image's end (not by the vendors' XMP, which differs), and copied out into a blob. *Keeping:* keepers hold every video; a companion holds it when it keeps the original; a viewer streams it (`fetch_now` kind `motion`). "Free up space" frees it with the original, once a keeper holds it. It is kept while its photo is (Recently Deleted included). *Not done:* a library imported before this has the `.MOV` of an earlier pair as a video of its own (not merged afterwards); shared albums share the still only; RAW+JPEG pairs (the other half of F2.10); hold-to-play on touch (the viewer has a LIVE button); whether a device can play the video is the platform's call (HEVC in a MOV is not played everywhere); Android's `MediaStore` carries no pairing, so a Live Photo copied onto an Android phone comes in as its still only (M4-03). *From PhotoKit (P1-08):* an asset's `.pairedVideo` (`.fullSizePairedVideo` when the still was edited) is read beside the still under the still's name stem, and the same folder pairing makes it the photo's video; the same Live Photo from a Mac and from an iPhone are the same bytes, so one photo with one video. Checked with a fake library; not yet on a real one (see A-121).

- **A-110 · W1-02, W1-03** · **Blob encryption for Shoebox Cloud and the share manifest.** *Cloud blobs:* `k_b = BLAKE3-derive(secret ‖ plaintext hash)`; XChaCha20-Poly1305 over 4 MiB pieces, each with a nonce derived from `k_b`, the piece number and a last-piece flag, so a blob cut at a piece boundary or extended does not open; `cid = blake3(ciphertext)`. No random nonce: the key is unique to the content, so the same piece is never sealed twice with different data. The cloud secret is `derive_key("shoebox/cloud-secret/v1", root seed)`, so the 24 words still restore everything. A 1 GiB seal and open peaks at 10.7 MB resident (`cargo test -p shoebox-core --release --test cloud -- --ignored`, the test binary measured alone on the Mac). *Manifest:* encrypted under a random per-share key with a random nonce (golden tests pass one in), header as associated data, signed by a device key of the sharer; the viewer takes the sharer's public key from the link, so a server that swaps in another manifest, share id or version fails the signature, and a link with a wrong key fails to open and says so. 50,000 items are 9.2 MB sealed (about 184 bytes an item with an original and a motion video on some): under the 10 MB budget but not far under, so a very large album would need paging or a smaller item before it gets bigger. Encode 52 ms and decode 27 ms in a release build on the Mac (not measured on a phone). The link's text form (how the key and the sharer's public key are written in the fragment) is not decided yet: W1-05 and W1-06. `chacha20poly1305 0.10` is the only new dependency (licences checked with `cargo deny`).

- **A-111 · W1-01** · **Web viewer: crypto in WebAssembly, iroh only for Live links.** `crates/shoebox-web` builds `shoebox-core`'s blob and manifest code for the browser (170 KB, 75 KB gzipped); the page never has a second implementation. `getrandom` has no browser backend by default, so the crate selects `crypto.getRandomValues` (target-specific dependencies and a `--cfg getrandom_backend="wasm_js"` in `.cargo/config.toml`). iroh built for the browser (5.2 MB, 1.56 MB gzipped, needs a clang with the wasm target for `ring`) fetched a blob through a relay in Chromium; this is for Live links only and loads on demand. **Not verified:** Safari, Firefox, any phone, video streaming by pieces, zips over 1 GB. Details in `docs/notes/web-viewer-spike.md`.

- **A-112 · W1-04** · **The cloud service.** `apps/cloud`, axum over the filesystem, everything indexed in memory at start. *Auth:* signed requests carrying the device's certificate chain (`shoebox_core::cloud_auth`); the service fixes a library's root key from the first request it sees while registration is open, so anyone who knows a library id before its owner registers could squat it (W1-10 gates registration on a subscription or a free-tier token; until then the id is the secret). A revoked device is refused only after the service has been sent the revocation. A blob upload's signature commits to the blob id and an object's to `X-Content-Blake3`, so a large body is never read for an unauthenticated caller. *Not built:* an object store backend, quotas per library (one number for all), a request log, TLS (run it behind a proxy we control). *Not verified:* behaviour at a million blobs, and the service under real network conditions; tested through the router in memory and by a smoke run on a socket. Details in `docs/notes/cloud-service.md`.

- **A-113 · W1-05** · **Sharing an album from a device.** `LibraryNode::create_web_share`, `revoke_web_share`, `rotate_web_share`, `refresh_web_shares`, `web_shares`, `push_cloud_certs` (`crates/shoebox-node/src/web_share.rs`); the link is `<service>/a#c.<share id>.<share key>` (base64url, `ShareLink`). *The cloud secret:* the root key is not kept, so the secret cannot be derived on demand: `create_identity` and `restore_from_kit` write it to the library setting `cloud-secret` (every member device then has it through the library, like the photo records), and an existing library enables it by typing the recovery phrase once (`enable_cloud_with_phrase`). Setting keys may not contain `/`, hence `cloud-secret`, `cloud-url`, `web-shares-<album>`. *The share key is stored in the library setting too* (this changes the earlier plan, which said it would not be), because a keeper that refreshes a share has to seal the new manifest with it. *Signing:* the manifest is signed by a key derived from the share key, not a device key, so a refresh by another device still verifies against the link; since the link also opens the manifest, the signature is an integrity check, not proof of who sent it. *What goes up:* thumbnails and previews of every photo in a personal album (a photo whose thumbnail or preview is not on this device yet is skipped and counted); videos always in full when they are 256 MiB or less and their location can be removed, otherwise withheld and counted; photo originals only with `full_size`, with the location removed by `strip_location` where the format allows and withheld where not (HEIC); a motion photo's clip with its location removed. A thumbnail or preview of a photo with a place is checked for one too. *Refresh:* a share is stale when its album's photo ids or their deleted state, or the full-size option, differ from the fingerprint stored with it; any device can refresh, only an explicit call does so today (a keeper doing it by itself is W1-09), and two devices refreshing together both write the same content, so the loser's write is harmless. Ciphertext ids are cached per plaintext hash in `cloud-cids.bin`, so a second link to an album uploads nothing but a manifest. A revocation is pushed to the service in the background after `revoke_device`, and again whenever shares are refreshed. *Not done:* shared (M5) albums; progress events for a UI; a request log. *Verified:* a real library node against the real service over a socket (`apps/cloud/tests/sharing.rs`): the link alone opens the manifest and every blob; 2 of 6 originals had a place removed in the fixture run; nothing of any photo is in the service's data directory. Not verified on a phone.

- **A-114 · W1-06** · **The viewer page.** `apps/web-viewer` (plain JavaScript modules, no framework and no build step beyond `scripts/build-viewer.sh`, which builds `crates/shoebox-web` to WebAssembly with wasm-bindgen 0.2.129 and copies the page's four files to `dist/`; the plan said Svelte, and a framework was not worth its weight for one screen). It parses the link with the same code the apps use (`parse_link`), opens the manifest (`open_share`), fetches each blob, checks it against its id and decrypts it in WebAssembly. Grid by day with lazy thumbnails (six at a time), a `<dialog>` viewer with swipe, arrow keys and focus return, motion clips and videos (videos up to 100 MB are decrypted and played from a Blob URL, larger ones download only; no seeking by pieces yet), download of one (the original when the share has it, else the preview) and a store-only zip of all (streamed to a file where `showSaveFilePicker` exists, in memory otherwise, with a prompt above 1.5 GB). The service serves it with `default-src 'none'`, `script-src 'self' 'wasm-unsafe-eval'`, `connect-src 'self'`, `img-src 'self' blob: data:`, `media-src blob:` and no inline script or style, and `Referrer-Policy: no-referrer`. *Checked in Chromium (the app's browser pane, hidden) against a real share served by the real service with 10 photos:* the manifest opens, all thumbnails load, the viewer opens, steps with the button and the keyboard, downloads the original with a date name, closes and returns focus; a link that is cut short, one with a wrong key and one for an unknown share each give their own message (the expired, busy and out-of-date messages are written but only the service side of expired is tested); the page's network log shows requests to its own origin only; the zip writer's output passes `unzip -t` and the standard CRC check value. *Not checked:* the 5,000-photo budgets (first screen under 2 s on a mid phone over 4G, under 200 MB while scrolling), Safari, Firefox, a phone, a formal accessibility audit (the markup uses real buttons, labels and a dialog, and nothing more has been verified), and the live download-all in a browser with `showSaveFilePicker`.
- **A-115 · W1-07** · **The share sheet.** *Share a link* on an album opens `WebLinkSheet`: a first-time step for the service address and the recovery phrase (typed once, since the root key is never kept), then the album's links with copy, QR, *new link* and *stop sharing*, a full-size toggle (HEIC, HEIF and AVIF originals are withheld because their location cannot be removed yet, and the sheet says so), an expiry choice, and the upload size shown before anything is sent. A link whose album has changed shows as out of date with a refresh button (`web_share_is_stale`). Commands: `web_links`, `web_link_check`, `web_link_create`, `web_link_revoke`, `web_link_rotate`, `web_links_refresh`, `cloud_setup`. *Checked:* the Rust command layer builds, bindings regenerate and the app's 66 library tests pass, and `npm run check` has no errors or warnings. *Not checked:* the sheet has not been opened in a browser or on a phone, and the Live choice is absent until W1-08.
- **A-116 · W1-08** · **Live links.** A Live link is `…/a#l.<share id ‖ one to four endpoint ids>.<key>`: it names the devices that may serve it (this device first, then the library's other devices, as they were when it was made) and carries no ticket or capability of its own, because the share key already is the read capability and the sealed manifest is signed with a key derived from it. The viewer page is still hosted by the service (it holds nothing for a Live link), so a Live link needs the service's address and the recovery phrase typed once, as a Cloud link does (the blob keys come from the cloud secret). A device that makes a Live link seals the album into `<data dir>/live/<share id>/` (the same blobs and manifest Cloud would hold; `manifest`, `meta` and one file per blob id) and answers `shoebox/webview/0` from it: one request per stream (manifest, or a range of a blob of at most 8 MiB), with reply codes for not served, expired, busy and bad request. It serves only the folders it holds, only while the share is still in the library's settings (revoking deletes the record and the folder, and another serving device deletes its folder at its next request) and before it expires; 16 streams at once and 1,500 requests per share per minute. A device that did not make the link serves it only when asked (`keeper web serve`, which also seals again links whose album changed); there is no automatic adoption yet (W1-09). A browser is not in the library's device list, so the connection gate lets `shoebox/webview/0` through for non-members like pairing. The page loads the iroh client (`crates/shoebox-web-live`, 3.3 MB, loaded only for Live links) and connects through the relay by endpoint id; the service's page policy must allow the relay hosts, **and iroh writes them with a trailing dot** (`relay.n0.iroh.link.`), which `*.iroh.link` does not match, so the operator passes both spellings with `--csp-connect` (found by trying it: the first run was blocked). *Checked:* node-level tests with a second node standing in for the browser (manifest, a thumbnail in pieces, other ids, a blob the share does not list, a path trick, a bad request, revoke, expiry, an album that changed, adoption on request); and **in Chromium (the app's browser pane) against a real app core with the n0 relay on, on this Mac: a Live link made from the share sheet opened, all 12 thumbnails loaded and a photo opened; with the app stopped the page said the album is not available right now and offered Try again; after restart Try again worked; after Stop sharing the page said the link stopped working.* *Not checked:* Safari, Firefox, phones; time to first photo is a few seconds to under a minute here (about 15 s the first time, about 45 s to the thumbnails after the app restarted) and has not been measured properly; relay traffic for 200 thumbnails is **estimated** at about 3 MB from the sealed sizes (a thumbnail is 10 to 20 KB, a preview 65 to 125 KB) and not measured at the relay; the n0 relays are not ours and are rate limited (V2-04 first); several serving devices answering one link has been tried only with one; full-size photos and videos over Live (ranges of large files) have not been tried in a browser; the share sheet's states are "from this device" and "from another device" only, not a live online/offline indicator for other devices.
- **A-117 · W1-09** · **The cloud keeper.** `LibraryNode::cloud_backup(policy)` seals each blob of the library under its own content key (the same objects a web link serves, so a share of a backed-up album uploads only a manifest) and uploads what the service lacks, newest photos first, in batches of 32, remembering what the service confirmed in `cloud-catalog.bin` (trusted for a week, then asked again). Originals go up as they are, location included: it is the person's own copy, not a share. The policy is two knobs, originals or not and a byte budget; *when* to run (a keeper overnight, a phone on Wi-Fi while charging, mobile data only if allowed) is the caller's, and only the keeper's loop exists so far (every 30 minutes: Cloud links, Live links, then the upload if `cloud-backup` is on, which the person turns on with `keeper web backup on`). Two small objects are stored last, each sealed under a key from its own content and named by that content's hash (`catalog.<hash>`: plaintext hash, ciphertext id, size, kind; `state.<hash>`: `LibraryStore::export_state`, a Loro snapshot), so a restore never finds a catalog that names blobs not yet there; the service's object names may now be 80 characters. `cloud_restore` folds the state in (`import_state`, with the checks a received change gets) and downloads every catalog blob this device lacks, checking the ciphertext against its id and the opened bytes against the hash the library knows, and keeps nothing that fails either. **Free up space** counts an original confirmed on the service within a week as another copy (`freeup.rs`), and the report names the photos for which it was the only one (`cloud_only`), so the confirmation can say so in its own words; a keeper still never frees space. *Checked in tests against the real service over a socket:* an upload, a second run that uploads nothing, a budget that stops and a next run that finishes, a restore of a photo's original, preview and thumbnail after they were collected from the device (bytes equal), the state folded back in, a phone with no other copy freeing nothing until the cloud copy exists and then freeing exactly those photos with `cloud_only` set, then restoring one. *Not done or not checked:* the encrypted copy does **not** yet count in the copy-count badge or the at-risk list (holdings are published per device; a cloud holder needs a place in the format) and "Free up space" in the app has no wording for it yet; no app screen or phone upload policy (Wi-Fi, charging, mobile data); a restore on an **empty machine from the recovery kit alone** is not exercised (it needs `restore_from_kit` and the library's epoch handling to meet the downloaded state; only a device that is already a member restores today); 10,000 photos (the issue's size) was not run, and the catalog is one object rewritten whole each time, which will want splitting; killing the network mid-upload was exercised only as a stop at a batch boundary, not by cutting a transfer; Recently Deleted purge does not yet remove cloud blobs (and blobs a live share lists must stay until that share goes); a revoked device's refusal is the service's rule from W1-04, not re-tested here.
- **A-118 · W1-10** · **Free and open, with a way to switch a link off.** Decided with the owner (2026-11-03): Shoebox Cloud runs free and open for now, with no subscription, payment, plans or lapse timeline (the part of W1-10 left for later; `--closed` still exists to refuse unregistered libraries). Every library has the same quota (`--quota-gib`, 2 by default), and a library over it is told in words what to do. Abuse is handled in the smallest way that works for an open service: the viewer page has *Report this album*, which sends `POST /report/{share}` with a few words (at most 200 characters, control characters removed; 5 reports per share and 120 for the service a minute); the service switches that share off at once (`410 reported`, ciphertext and record kept, a later refresh of the share does not turn it back on), the owner's app shows it as switched off with the reporter's words, and the operator either deletes the share or reinstates it by deleting `reports/<share id>` in the data directory. **Weakness, on purpose and said plainly:** anyone who holds a link can switch it off, which is also a way to annoy a sender who forwarded it; the owner can make a new link. Live links are not reported through the service (it holds nothing for them), so the page offers no report there; the sharer stops those themselves. *Checked:* a service test (report, both public routes answer 410, the owner's list carries the reason, a refresh does not undo it, reinstating works, unknown share is 404). *Not done:* a written policy and contact address (the page says nothing about who to write to); repeated offences suspending a library; bot controls beyond the rate limits; telling the owner outside the app; per-library quotas that differ; no personal data beyond the library's keys is stored, but a test that lists exactly what is was not written; the report button and the owner's "switched off" line were not opened in a browser.
- **A-119 · W1-04 follow-up** · **Shoebox Cloud on a bucket.** The service can keep every byte in an S3-compatible bucket (`--s3-endpoint`, `--s3-bucket`; R2, S3, B2, MinIO) and keep only bookkeeping on its own disk, so it can run on the smallest machine and the bucket carries the traffic. Presigned with `rusty-s3` (no SDK). *Uploads* are three steps for every client, bucket or not: `POST /v1/uploads` reserves quota and hands out a URL per blob or object, the app `PUT`s the bytes there, `POST /v1/uploads/commit` records them; the old signed `PUT /v1/blobs/{cid}` and `PUT /v1/objects/{name}` are gone (the apps and the service ship together; there is no deployed service to stay compatible with). *Reads* are `302` redirects to presigned GETs, so the viewer page is unchanged and its CSP picks up the bucket's origin. **What changes for a person, said plainly:** (1) stopping a link stops new reads at once, but a blob address the page already got keeps working for up to 10 minutes (the presigned lifetime) and may sit in the browser's cache for 5; (2) the per-share daily limit is counted when a redirect is handed out (the whole blob each time), not on bytes actually sent, so a browser that re-asks for a range is charged again; (3) the bucket checks an upload's size (signed `Content-Length`, then a HEAD at commit) but not its BLAKE3 hash, so a broken or dishonest app can store a wrong blob under a right id; readers already check every blob against its id, so it only spoils that library's own links and restore; (4) an upload made but never committed is deleted by the janitor (every 10 minutes) after its two hours; (5) the data directory (`index.log` per library, share records, reports) must be backed up, since the bucket alone cannot be served; (6) one service per data directory and bucket prefix. *Checked:* service tests against a fake bucket that verifies every presigned signature, its expiry and that no `Authorization` header reaches it (uploads go straight there, reads redirect, a wrong size, a late PUT and a never-made PUT are not recorded, a restart keeps the index, an expired share's manifest leaves the bucket); the node's sharing and cloud-keeper tests through the fake bucket; **and against MinIO (an independent SigV4 implementation)**: `with_a_real_bucket_a_link_opens_and_the_library_restores` (ignored; `SHOEBOX_TEST_S3_ENDPOINT` and keys) and the viewer page in Chromium (the app's browser pane): the manifest and all ten thumbnails came by redirect from MinIO, a photo opened, no CSP or CORS errors. *Not checked:* R2, AWS S3 or B2 themselves (their CORS rules and `auto` region), large objects (no multipart upload, so one object is at most 5 GiB on S3 and the service caps at 512 MiB anyway), and phones over the new upload path.
- **A-120 · W1-04 follow-up** · **Shoebox Cloud deploys to Cloudflare or to a server of your own.** The service's records (libraries, shares, reports, uploads in progress) go through a key-value interface, `meta::Meta`, and its calls to the bucket through `s3::Http`, so the same service code runs natively (files in a data folder, reqwest) and in a Cloudflare Worker (`apps/cloud-worker`: a Durable Object's SQLite database, `fetch`); `apps/cloud` without its default `server` feature builds for wasm32. Uploads in progress are now records, so a restart (or an evicted Durable Object) keeps their quota reserved and the janitor still deletes what was never committed. An operator interface (`/admin`, with a bearer token) lists reports and reinstates or deletes a share. *On Cloudflare:* one Durable Object, `main`, runs the whole service, so every write is in order without locks; the front Worker serves the viewer from static assets with the same CSP, and follows the service's redirect to R2 itself, streaming the bytes, so a reader never holds a bucket address: **stopping a link stops it at once** (better than A-119's 10 minutes) and R2 needs no CORS rule. **What this costs, said plainly:** (1) the object keeps every record in memory, within a Worker's 128 MB, so one deployment holds roughly a few hundred thousand blobs across all libraries before it needs splitting (one object per library would be the next step); (2) when the object wakes after being idle it reads all its records again, so the first request after a quiet spell is slower; (3) the per-address and per-link rate counts live in memory and start again when the object is evicted (the daily byte limit too), so they are looser than on a server; (4) every API call goes through the one object, which serialises them; bytes do not, since uploads go straight to R2 and reads stream through the front Worker. *Self-hosted:* `deploy/selfhost` (a non-root Docker image with the viewer, compose with Caddy for HTTPS, an example `.env`, a hardened systemd unit); every flag has an environment variable. *Checked:* the service tests (17, including the operator calls and a service reopened mid-upload on in-memory records) and sharing tests; **the same end-to-end test (`with_a_deployed_service_…`, ignored, `SHOEBOX_TEST_CLOUD_URL`) passed against the Worker in `wrangler dev` with MinIO as the bucket and against the native binary configured only by environment variables**: upload, open the link, a range, report, list and reinstate as the operator, stop the link, restore the library; the Worker-served link opened in Chromium (the app's browser pane) with no console errors and `connect-src 'self'`; a link kept across a restart of `wrangler dev` opened again from the object's database. Two Worker bugs were found that way: the forward to the Durable Object followed the service's redirect itself (now `redirect: manual`), and the janitor's alarm was set for 2083 (`set_alarm` takes an offset). *Not checked:* a real Cloudflare account and R2 (the R2 S3 API from inside a Worker, the custom domain), the Docker image build (no Docker daemon here; `docker compose config` passes), the systemd unit, and a Durable Object evicted in production rather than restarted locally.
- **A-121 · F2.1 · P1-03/04** · **The Mac's Photos library as a backup source.** *ShoeboxPhotos* (`apps/desktop-mobile/native/ShoeboxPhotos`) is the one PhotoKit implementation for the Mac and, later, the iPhone (P1-06): Foundation, Photos, AVFoundation and ImageIO only. The photo is the current version (D2: `.fullSizePhoto`/`.fullSizeVideo` when edited, else the original), read under the name it came in with. Changes come from `fetchPersistentChanges` with the token kept as `pk1:` + base64; a missing, foreign or unusable token means a full walk, whose next token is taken before the walk. Only the person's own photos are listed (not shared albums or hidden ones), and an asset that stops being listed is not taken as deleted: only a reported deletion is (A-092 rules, P1-02). Size and "in iCloud only" are read by key (`fileSize`, `locallyAvailable`), not in the headers; unknown availability counts as here, and a read without the network then says otherwise. Reads go to a file (`writeData`), so memory stays small. *On the Mac:* `macos_photos.rs` over C entry points that return JSON; `build.rs` compiles the package with `swiftc` directly (not `swift build`), because this Mac's Command Line Tools 26.5 carry a stale `PackageDescription` private interface from 2024 that no manifest compiles against; `test.sh` runs the unit tests the same way, and CI runs them on its macOS runner. Settings → *Back up the Photos library* is off by default; turning it on asks for access, then shows how many photos and videos and how many bytes, and how many are in iCloud only. Originals are copied in (D4); one in iCloud only waits for mains power (D3) and the card says how many wait; Wi-Fi, charger and folder rules do not apply. A deletion in Photos goes to Recently Deleted on the next scan (every 30 s), an edit adds the new version and drops the old. Turned off, it keeps what it imported and Photos is not looked at; a deletion made meanwhile is seen once it is on again (unlike a phone's gallery, which is followed with backup off, A-092). Poster frames of videos come from AVFoundation. *Checked:* the Swift unit tests (resource choice, names, MIME types, reply shape, tokens, change merging), the backup path with a fake library (Live Photos paired, iCloud-only waiting then downloaded, the preview and access refused), and a bundled debug app builds with the usage string. *Not checked:* anything against a real library: this Mac's Photos library is empty, so the P1-01/P1-03 numbers (20k assets under 10 s, stream hash against export, change kinds, the token across a restart, iCloud-only detection) are still to take, and a backup seen from a second device. Each rebuild is ad-hoc signed, so macOS asks for access again after one.
- **A-122 · M4-11** · **Library memory per replica: 4.9–5.9 KB a photo down to 2.0–2.5 KB** (`docs/notes/phone-memory.md`). The Loro document was most of it: compaction saved a full snapshot, and a document opened from one (or one that exported one) holds about three times what the same document holds when built from its updates. Replicas now compact to the update form (older snapshot files still open, tested), log imported bytes as they arrived instead of re-exporting them, and free Loro's diff and history caches after each import; a first sync into an empty replica skips the diff. `LibraryState` keeps photos packed (MIME types, camera names and device ids stored once) and a blob's holders in a sorted list. Costs: an incremental sync rebuilds the diff cache (8 → 36 ms for 100 edits at 100k); opening imports operations rather than a snapshot (no slower here, 1.7 s at 100k with signatures). *Not met:* the first sync of a large library still travels as one message and peaks several hundred MB above the result (batched sync is M4-11a); nothing was measured on a phone (M4-11c).
- **A-124 · S2-01** · **Indexing work is shared among whoever is indexing when no always-on device is.** `share_candidates` (`shoebox-core::policy`) picks the devices that share the backlog, the text and face reading, shard compaction and face grouping: the always-on indexers that are working (rate above 0) and can do the job, as before; with none, every working device that can. Before, a library with no keeper and no desktop set to index on power left every photo a phone had not imported unindexed for 48 hours, and nothing grouped faces or compacted shards. *Consequences, said plainly:* (1) a phone's share only moves while it charges, so with two phones one that rarely charges delays its share until the other takes it over after the 48-hour grace; (2) the device that groups faces is the lowest device id among the candidates, so a phone that seldom charges can delay grouping the same way (a takeover for grouping is not built); (3) a keeper that is paused hands everything to the phones, and takes it back when it resumes (shares move only by the rate it announces). *Checked:* policy unit tests; testkit with two phones and no keeper (300 photos shared 1:3 within 12%, no wait), a phone alone, compaction without an always-on device, face grouping choice with phones only, with a keeper and with a paused keeper; the new tests fail on the old code. *Not checked:* a real phone.
- **A-125 · S2-02** · **A phone with no other device downloads the search models once.** `ModelFile` (generated, `crates/shoebox-search/src/models.rs`) now carries the address of its published copy: a fixed GitHub release, `models-v1` (`MODELS_RELEASE` in `xtask/src/models.rs`; the release workflow's `models` job puts any file that release lacks there and never replaces one; a changed model is `models-v2` with new hashes compiled in). `LibraryNode::model_bytes_from` takes the first of: this device's blob store; a copy in the device's own folders (checked by size and hash, then offered to the other devices); a device of the library that offers it (a failure there falls through to the download); a download. A download is resumable (`.part` plus `Range`), stopped at the expected size, and kept only when its BLAKE3 hash matches the one compiled in, so the address is trusted for bytes only; it then goes into the blob store as search data and is offered to the others like any model. On Android the index job allows a download only on an unmetered network (`NET_CAPABILITY_NOT_METERED`), checked when the job starts; the job runs in a process of its own, so progress goes through `model-download.json` in the data folder, which Settings -> Search reads (stale after 90 s). *What the server learns:* that someone fetched a model file from GitHub (an IP address and the app's version), nothing about the library. *Checked:* tests with a local HTTP server (kept when right; refused with nothing kept when the hash is wrong; refused when longer; resumes from where a dropped connection stopped; restarts when the server cannot resume), the order of the three sources with real nodes, `cargo xtask models --check`. *Not checked:* a real download from GitHub, the release workflow's `models` job (it only runs on a tag), the Android job on a phone, the Settings line in the app. *Not done:* the desktop app does not download (a desktop without the models next to it does not index, as before); only Android uses it.
- **A-127 · S2-04** · **The Android index job runs until the phone has nothing left to do.** The indexer reports `caught_up` when a pass found no picture, text or face work for this device (gate open, not paused); a photo that cannot be read (decode fails) is remembered for the run and not retried, so it cannot keep a phone awake. `index_burst` ends on `caught_up`, the budget, or an `index-stop` file, then flushes, groups faces and compacts shards when the S2-01 rule picks this device. It writes `index-progress.json` (`indexed`, `total`, `left`, `done`) for the notification, since the job runs in its own process. `IndexWorker`: when the last run left more than 500 photos it is a foreground worker (dataSync service, progress notification, 3 h budget); otherwise a short burst of 8 min. A watchdog writes `index-stop` when the phone reports thermal status SEVERE or worse or is unplugged, and `onStopped` does the same when WorkManager cancels it, because the run is native code. A run that stopped with work left and the phone still cool and charging queues the next run at once (unique work, so they do not pile up); if Android refuses a foreground service from the background, the same chain of short bursts does the job. *Not changed:* previews, because a phone releases a preview only while it holds the original, and reads faces and text from whichever it keeps. *The app's process:* once the app has been opened and left, its process stays alive holding `app.lock`, and the job runs in that same process; it used to find the lock taken and skip, so nothing indexed. Now the app registers its core and the runtime it runs on (`set_app_core`), and the job borrows it (`index_burst` on that core, resuming the indexer that the last run paused) instead of opening a second one; the watchdog stops the run when the app comes to the foreground. The loaded models (about 150 MB) then stay resident in the app process until Android reclaims it. "Rebuild this device's search index" in Settings only rebuilds the local facts and vector index; it never re-indexes photos. *Not checked:* any of the Kotlin (no JDK here to compile it, CI does not build Android), the foreground service on a phone, the thermal and unplug stops, a 2,000-photo run on the S25 (not connected when this was written). The first run on a new phone is a short burst, since the progress file does not exist yet; the next one, 30 min later or at once if the burst chained, is the long one.
- **A-128 · S2-05** · **Settings → Search says how long a phone on its own will take.** When this device is the only one with a rate above 0, the line reads "indexing on its own while it charges: N photos left, about K nights", with a night taken as six hours of charging and the rate as the measured photos a minute (pictures only: text and faces come after and add to it). The tester and user guides drop the "image model comes from a computer or keeper" limit and say a phone alone downloads the models once and indexes by itself, flagged as not yet run for a night on a real phone. *Not done:* the S25 measurements (a 2,000-photo run: time, temperature, charge time) for `scale-endurance.md`, because the phone was not connected.
