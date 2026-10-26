# Assumptions log

Reasonable assumptions made while implementing the issues in `planning/issues/`, recorded for review.
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

- **A-001 · M0-01** · `shoebox-core` and the other crates use a `0.0.1` workspace version and `publish = false`.
  Why: nothing is released yet. Reverse: bump in `[workspace.package]`.
- **A-002 · M0-02** · `repository` is an `example.invalid` placeholder since no remote exists yet.
  Reverse: set the real URL in `[workspace.package]` once a remote is created.
- **A-006 · M0-03** · CI cannot run yet (no remote), so the workflow in `.github/workflows/ci.yml` is unexecuted;
  its steps were reproduced locally (`cargo fmt/clippy/test`, `cargo deny check licenses sources`,
  `scripts/check-no-app-deps.py`). The "deliberate AGPL dependency" acceptance check was verified locally
  with a registry AGPL crate (`lemmy_utils`) in `shoebox-core` → `cargo deny` failed; reverted.
  Note: cargo-deny skips *path* crates as "private", so only registry deps are covered by the allow-list; edges
  from libraries to `apps/*` are covered by `scripts/check-no-app-deps.py` (verified by giving `keeper` a temporary
  lib target). Bin-only apps can't be depended on anyway.
- **A-059 · M2-01** · Known flake: `shoebox-store-docs` `autosync::sync_now_waits_for_a_sync_already_in_flight` (300 records, `fast()` 10 s sync timeout) fails
  intermittently — 1 in 6 runs on the pre-M2 tree (verified with `git stash -u`), more often on a nearly full disk. It is a sync that hangs inside iroh-docs, unrelated
  to identity. Not fixed; re-run before treating it as a regression. Worth a proper look if it appears in CI.
  Second known flake (seen 2026-10-26, 2 in 10 runs on the code from before that day too): `shoebox-store-docs` `freeup::dry_run_and_real_run_agree_and_pins_are_kept`
  (the dry run's byte total sometimes differs from the expected one, `freeup.rs:78`). Not investigated.
- **A-065 · M2-09** · Release automation is written but never ran: `release.yml` (tag `keeper-v*`; musl x86_64 natively, aarch64 through `cross`, macOS universal via `lipo`; SHA256SUMS signed
  with minisign from repo secrets) and `scripts/install-keeper.sh` (verifies the *signature of SHA256SUMS* then the tarball checksum; refuses to install without `minisign`). The
  project public key is a placeholder (`MINISIGN_PUBKEY`) and the repo/owner is `OWNER/shoebox` until a remote and key exist. Not verified: musl/cross builds of the iroh stack, the macOS lipo
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
  "relay → direct after hole punching" check (needs two real networks).
- **A-062 · M2-04** · Pairing as built: ticket = `{inviter addr, 32-byte secret, expiry, role}` (`shoebox-pair1` + hex + checksum, same shape as the library ticket; this string is what a QR
  encodes). The secret authenticates the exchange (keyed-BLAKE3 proof over both ids and a joiner nonce; the ticket is found by a key-derived 8-byte id); the six-digit code is a keyed hash of
  secret, ids and both nonces — people compare it, and *either* side declining aborts before anything is written. The inviter consumes the ticket on a valid proof (single use), expiry
  uses the node clock (default offered 5 min by the caller), and failed lookups are rate-limited per joining device (30/min, then that device is refused for a minute; a global limit let any stranger lock pairing for everyone, changed 2026-10-26; per-ticket proof
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
  here. Not done: a system notification when the app is closed (the alert waits for the next launch); the keeper `status` does not list
  alerts; pairing a new non-admin device does not alert (it needs a ticket from inside). Root rotation followed (A-088). Tested:
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
- **A-064 · M2-08** · Free-up's "≥ target copies elsewhere" is implemented literally: other devices' *originals* (replicated holdings, excluding
  this device) must number at least the redundancy target (default 2) **and** at least one of those holders answered a `shoebox/has/0` probe for
  that blob within 7 days (confirmations persist in `holder-confirmations.json`; a holder that denies a hash is forgotten for it). Free-up probes
  holders itself, in dry runs too, so both runs agree. Pinned originals, blobs a shared album still references, and keepers (error) are excluded.
  A real run releases the library/pin protection and retracts holdings; the bytes disappear when the blob store's GC runs (reported bytes = sizes
  of the blobs released). A phone does not re-fetch freed originals only because its fetcher policy wants originals of pinned or own below-target
  photos, and freed photos have enough other copies; revisit if a pin/own policy ever overrides that.
  **Out of step with "backed up" since commit e72a2d9:** the at-risk count and each photo's `backed_up` flag treat one keeper holding the
  original as enough, whatever the target, but free-up still counts copies only (`others.len() >= target` in
  `crates/shoebox-node/src/freeup.rs`). So a photo on a phone and one keeper shows as backed up yet cannot be freed until the target is lowered
  to 1 or a third device holds it. The user guide describes this free-up rule ("at least two other devices"). Decide whether a confirmed keeper
  copy should also be enough to free space.
- **A-068 · A-060 closed** · Recovery without any capability: `Node::recover(link, phrase, role, name, now)` dials a surviving member on `shoebox/recover/0` (open to unlisted devices, like pairing). The member
  sends a challenge (nonce + the library's identity id), the device answers with a root-key signature over nonce/its id/identity plus an admin cert and a device cert for itself signed with the root key; the
  member checks the key is the library's root key and both certs chain to it, lists the device (as admin) and returns the current library capability. A wrong phrase changes nothing; failures are rate limited
  per device (10/min, then that device is refused; global until 2026-10-26, which let any stranger block recovery). The device is found through a `PeerLink` (`shoebox-peer1…`, no secret) that a surviving device shows (`LibraryNode::peer_link`, `shoebox-keeper address`). Not covered: finding a
  surviving device automatically (mDNS/relay lookup by library) and recovery when *no* device survives (the data is gone by design; the kit only restores identity). Recovering onto a rotated library works (the
  member answers with its current epoch). `shoebox-keeper recover` is the CLI entry.
- **A-083 · M5 not done** · Shared albums need a library on the device (no "shared albums only" mode, M5-02). No gossip presence, nudges or avatars (names are shared, M5-06). The accept-invitation screen has no storage-cost line. The UI loads an album page by page (500) but not virtualised. Notification posting when someone adds photos is not wired.

### Rules for future changes

Not open questions, but they must be honoured by any format or protocol change, and re-checked on every iroh upgrade (shared albums still run on iroh-docs).

- **A-008 · M0-04** · Record encoding is `[version byte] ++ postcard(body)` with the version defined per type
  via `Record::VERSION` (all `1`). There is no type-tag byte; the *key/namespace* identifies the record type.
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
- **A-084 · M6 decisions** · (1) *Format marker:* `format.json` in the app data dir and the keeper storage dir (layout 1 = everything as of this commit; layout 0 = data without a marker, adopted with a backup). The backup copies files up to 64 MB outside `blobs/`, `posters/` and `backups/`, so it holds the small state (app.json, config, registries, Loro snapshot/log up to that size) but not the blob store or a very large docs database; future migrations that rewrite those must carry their own safeguard. (2) *N and N-1:* protocols are named with a version (`shoebox/loro/0`); a release that changes one serves old and new for a release. Nothing has needed a second version yet, so the rule is tested (names carry versions, an unserved version is refused cleanly) but not yet exercised by a real change. (3) *Diagnostics:* the log tail is an in-memory ring (400 lines, INFO and above) installed by `init_logging`; redaction is by token (paths, file names, coordinates with four or more decimals, e-mail addresses, invitations, long hex) plus the names this person gave things, and errs on the side of removing too much (a plain decimal like `1.2345` becomes `<coordinate>`). The report is shown for copying or saving; the OS share sheet is not wired. (4) *Accessibility:* contrast was checked against WCAG AA (4.5:1) for the design tokens and the failing ones were darkened (ink-3, accent, ok, warn in light; ink-3 in dark), so the app is slightly darker than the mockups; *Text size* uses CSS `zoom` on the page because several webviews ignore the system font size; screen-reader behaviour is untested. (5) *Failure scenarios:* two libraries that merely share a capability, without certified devices, do not exchange records (found while writing the clock-skew test); last-writer-wins follows the hybrid clock, so a clock a day ahead wins conflicts until the others catch up.

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
- **A-060 · M2-02** · The kit holds the *root key* (24 words = its 32-byte seed as BIP39 entropy, English list; typed phrases are case/space-forgiving) plus library id and
  creation date in a printable text file (`RecoveryKit::to_text/from_text`). `LibraryNode::create_identity(root, …)` returns the kit and keeps nothing (the issue's
  `Node::export_recovery_kit()` cannot exist because the root is never stored; export happens once, at creation). `restore_from_kit(phrase)` verifies the phrase against the
  library's root key before writing anything, then certifies the new device as admin member. **Gap (acceptance "rejoins and sees all photos once a keeper is reachable")**:
  the kit does not contain the library's *space capability*, so a device that lost everything cannot yet fetch data; this needs a root-proof handshake in which a surviving
  member/keeper hands the capability to a device that signs a challenge with the root key — scheduled with the pairing protocol (M2-04). `recovery_kit_saved` is the
  replicated setting `recovery_kit_saved` (`[1]`).
- **A-071 · M3-03** · TypeScript bindings come from `ts-rs` (`#[ts(export)]` on the shared types, written to `ui/src/bindings` by `cargo test`, `git diff --exit-code` in CI) instead of `tauri-specta`:
  the command surface is one generic `command(name, args)` shared with the HTTP bridge, which specta's per-command generation does not fit. Argument shapes are checked at run time (`commands.rs`
  `arg`/`opt`), results are typed on the TS side. A mismatch in a *result* type fails the build; a mismatch in an *argument* only fails when called. 64-bit numbers cross as JSON numbers
  (`f64` in the bindings): fine for millisecond timestamps and sizes, not for ids (those are hex strings).
- **A-081 · M5-05 removing a member** · The owner writes a new namespace with the same album id, items, activity and the members that stay, and pushes `Moved{capability, owner}` to each of them over `shoebox/album/0` until acknowledged (retried every few seconds while the owner runs). A member accepts only if the sender is the inviter it joined through (only the owner invites); until 2026-10-26 it checked the membership entries, which any contributor can forge, so a contributor could make itself "owner" and move everyone, the real owner included, to a space it controlled. The owner's own device has no inviter, so it never follows a `Moved`. Contributors get the write capability, viewers the read capability. Viewers cannot write a membership entry, so the owner records them when they present their invitation (`viewers` in the registry; they are shown as "Viewer ab12cd34"). A member that is offline when the owner is online follows when both are next online; until then it keeps working in the old space, where nobody who stayed will see its additions. The removed member keeps the photos they downloaded, still has the old namespace (frozen), and is dropped from the owner's blob gate at once. The owner cannot be removed and leaving as owner leaves an album nobody can invite to.
- **A-082 · M5 limits and scale** · 50 members and 50,000 photos are enforced when inviting and adding (clear messages). Writing 50,000 items takes about a minute on a laptop (`cargo test -p shoebox-store-docs --test album_reopen -- --ignored`); the "50k items to a new member in under 2 minutes" criterion is covered by `album_sync_of_50k_items` (ignored, run on demand).
