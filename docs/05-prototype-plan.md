# 05 · Technical plan

The technical plan for the MVP. The first milestones prove the core loop on real hardware: **import → sync metadata → fetch blobs by role →
see copy counts → pair a keeper → share an album**. Later milestones turn it into a shippable MVP.

> iroh API names below were checked in **M0-01** against the pinned 1.x line
> (`iroh 1.4.1`, `iroh-blobs 0.103.1`, `iroh-gossip 0.101.0`, `iroh-docs 0.101.0`).
> Versions, corrected names and gaps: [`docs/notes/iroh-versions.md`](notes/iroh-versions.md).
> Notable renames: "discovery" is now *address lookup* (mDNS is the separate crate `iroh-mdns-address-lookup`),
> and docs entries store blob hashes (content travels over iroh-blobs).

---

## 1. Workspace layout

One Cargo workspace. The app and the keeper share the same `shoebox-node` facade; they differ only
in how they host it.

```
shoebox/
├── Cargo.toml                    workspace; pins iroh, iroh-blobs, iroh-gossip, iroh-docs
├── crates/
│   ├── shoebox-core/             pure domain: types, traits, policies. No iroh, no I/O.
│   │   ├── model.rs              PhotoId, PhotoRecord, Album, DeviceCert, Holding, Role, Hlc
│   │   ├── store.rs              LibraryStore / AlbumStore traits (§2)
│   │   ├── identity.rs           root key, device certs, signed device list, recovery kit encoding
│   │   ├── policy.rs             role → fetch plan, redundancy math, "safe to free" rules
│   │   └── merge.rs              LWW / add-wins rules shared by every backend
│   │
│   ├── shoebox-store-docs/       LibraryStore on iroh-docs            (prototype default)
│   ├── shoebox-store-loro/       LibraryStore on Loro + gossip/blobs  (spike, feature-gated)
│   ├── shoebox-store-mem/        in-memory impl for unit tests and the conformance suite
│   │
│   ├── shoebox-media/            import pipeline: BLAKE3 hash, EXIF (kamadak-exif), thumbnail/preview
│   │                             encode (image + libheif via feature), video poster frame (later)
│   │
│   ├── shoebox-node/             the runtime: owns iroh Endpoint + Router, blob store, a LibraryStore,
│   │   ├── net.rs                endpoint setup, relay config, mDNS/local discovery
│   │   ├── pairing.rs            ALPN "shoebox/pair/0" protocol + SAS (§4)
│   │   ├── presence.rs           iroh-gossip topic per library / album: online, profile, "new item"
│   │   ├── fetcher.rs            role-driven blob fetch queue, holder selection (LAN → direct → relay)
│   │   ├── holdings.rs           publishes what this device holds; computes copy counts
│   │   ├── sharing.rs            album spaces, invite tickets, location-strip derivation
│   │   └── api.rs                `Node` facade + event stream used by the keeper and the app
│   │
│   └── shoebox-testkit/          spin up N in-process nodes, scripted scenarios, fake clocks
│
├── apps/
│   ├── keeper/                   `shoebox-keeper` binary (clap): pair, status, run, service install
│   └── desktop-mobile/           Tauri 2 app
│       ├── src-tauri/            commands + events over `Node`; mobile plugins (§6)
│       └── ui/                   web frontend (Svelte 5 + Vite), ported from design/mockups
│
├── tools/
│   ├── store-bench/              100k / 500k-entry LibraryStore benchmark (decides #7, issue M1-15)
│   └── shoebox-dev/              developer CLI for M1–M2 demos before any UI exists (M1-07)
└── xtask/                        cargo xtask: fixture corpus generation (M0-08), release helpers
```

**Rules**
- `shoebox-core` has no async runtime or iroh dependency. That keeps the domain logic testable, and it is what makes the store swap cheap.
- Each backend must pass the same **conformance suite** (`shoebox-core::store::tests`, run against `mem`, `docs` and `loro`).
- `shoebox-node` is the only crate that knows about iroh. The apps only see `Node`.

---

## 2. `LibraryStore` trait

The trait sits at the **domain level** (photos, albums, devices), not as a generic KV store. That way a CRDT
backend can model things natively rather than emulating KV. Merge rules are defined once in `core::merge`,
and every backend has to produce the same results.

```rust
// shoebox-core/src/store.rs  (sketch)

pub type PhotoId = Blake3Hash;            // hash of the original bytes
pub struct EndpointId(pub [u8; 32]);      // iroh endpoint public key

pub struct PhotoRecord {
    pub id: PhotoId,
    pub original: BlobRef,                // hash + size + mime
    pub preview: BlobRef,
    pub thumb: BlobRef,
    pub taken_at: Timestamp,
    pub exif: ExifSummary,                // camera, lens, dims, optional GPS
    pub imported_by: EndpointId,
    pub favorite: Lww<bool>,
    pub deleted_at: Lww<Option<Timestamp>>, // tombstone; purge after 30 days
}

pub struct Holding { pub device: EndpointId, pub blob: Blake3Hash, pub kind: BlobKind, pub since: Hlc }

pub enum Change {
    PhotoUpserted(PhotoId),
    PhotoDeleted(PhotoId),
    AlbumChanged(AlbumId),
    DeviceListChanged,
    HoldingsChanged { device: EndpointId },
    RemoteSyncProgress { peer: EndpointId, done: u64, total: Option<u64> },
}

#[async_trait]
pub trait LibraryStore: Send + Sync + 'static {
    // --- photos ---
    async fn upsert_photo(&self, rec: PhotoRecord) -> Result<()>;      // idempotent by id; merge()
    async fn photo(&self, id: &PhotoId) -> Result<Option<PhotoRecord>>;
    async fn photos_by_time(&self, range: TimeRange, page: Page) -> Result<Vec<PhotoRecord>>;
    async fn set_favorite(&self, id: &PhotoId, fav: bool) -> Result<()>;
    async fn mark_deleted(&self, id: &PhotoId) -> Result<()>;
    async fn purge_expired(&self, now: Timestamp) -> Result<Vec<PhotoId>>; // returns blobs to GC

    // --- personal albums ---
    async fn upsert_album(&self, album: AlbumMeta) -> Result<()>;
    async fn album_add(&self, album: &AlbumId, items: &[PhotoId]) -> Result<()>;    // add-wins set
    async fn album_remove(&self, album: &AlbumId, items: &[PhotoId]) -> Result<()>;
    async fn albums(&self) -> Result<Vec<AlbumSummary>>;

    // --- devices (signed by library root; store only persists + verifies) ---
    async fn device_list(&self) -> Result<SignedDeviceList>;
    async fn apply_device_change(&self, change: SignedDeviceChange) -> Result<()>; // rejects bad sigs

    // --- holdings / redundancy ---
    async fn put_holdings(&self, device: EndpointId, delta: HoldingsDelta) -> Result<()>;
    async fn holders(&self, blob: &Blake3Hash) -> Result<Vec<Holding>>;
    async fn at_risk(&self, min_copies: u8, page: Page) -> Result<Vec<PhotoId>>;

    // --- sync plumbing ---
    fn subscribe(&self) -> BoxStream<'static, Change>;
    async fn sync_with(&self, peer: EndpointAddr) -> Result<()>;       // backend decides how
    fn capability(&self) -> Capability;                                 // Write / Read
}

/// Shared albums are separate spaces with their own membership + capabilities.
#[async_trait]
pub trait AlbumStore: Send + Sync + 'static {
    async fn meta(&self) -> Result<SharedAlbumMeta>;
    async fn add_items(&self, items: Vec<SharedItem>) -> Result<()>;   // SharedItem has original + optional sharedHash
    async fn items(&self, page: Page) -> Result<Vec<SharedItem>>;
    async fn members(&self) -> Result<Vec<Member>>;
    async fn events(&self, since: Option<Hlc>) -> Result<Vec<AlbumEvent>>; // activity feed
    async fn invite(&self, role: MemberRole, expiry: Expiry) -> Result<InviteTicket>;
    fn subscribe(&self) -> BoxStream<'static, Change>;
}

/// Backend entry point: create/open/join spaces.
#[async_trait]
pub trait StoreBackend: Send + Sync + 'static {
    type Library: LibraryStore;
    type Album: AlbumStore;
    async fn create_library(&self, root: &RootPublicKey) -> Result<(Self::Library, LibraryCapability)>;
    async fn open_library(&self, cap: &LibraryCapability) -> Result<Self::Library>;
    async fn create_album(&self, meta: SharedAlbumMeta) -> Result<Self::Album>;
    async fn join_album(&self, ticket: &InviteTicket) -> Result<Self::Album>;
}
```

**Merge semantics** (in `core::merge`, identical for every backend)
- Every mutable field is LWW, ordered by a hybrid logical clock (HLC) with the device id as tie-breaker.
- Album membership is an add-wins set. A concurrent add and remove of the same item → it stays in.
- Tombstones win over edits made at or after the delete. Restore from Recently Deleted is an explicit LWW write.
- Holdings are written only by the device they describe (single writer per key, so no conflicts).

**How iroh-docs maps onto it:** one namespace for the library and one per shared album. Keys follow the
data model in `03-architecture.md`. Values are postcard-encoded records. The LWW logic runs in `core::merge`
on read, because iroh-docs' own last-write-per-author semantics aren't enough for multi-device fields.

---

## 3. Blob handling

- One iroh-blobs file store per node (`<data>/blobs`). Originals imported from the camera roll on mobile are
  **referenced in place where possible** (no second copy) and copied only if the OS file can disappear.
- Fetch plan (`core::policy`):
  - **Keeper**: thumbnails → previews → originals, all of them.
  - **Phone**: thumbnails → previews for everything, plus originals only when pinned.
  - **Viewer**: thumbnails → previews.
- Shared albums: previews only, unless that album's "Keep originals" is on (F6.9).
- Holder selection: holdings entries ∩ currently connected peers, ranked LAN > direct > relay, then most recently seen.
- "Free up space" (F7.3) deletes a local original only if `holders(original)` shows ≥ target copies *on other
  devices* **and** one of them confirmed the hash within the last 7 days (through a lightweight `has` probe).

---

## 4. Pairing protocol (`shoebox/pair/0`)

1. The existing device (or the phone, for keeper pairing) creates `PairTicket { addr: EndpointAddr, secret: [u8;16], expires }`.
   It's encoded as a QR code and as a text string.
2. The joiner dials `addr` on ALPN `shoebox/pair/0` and sends `HMAC(secret, joiner_id)` to prove it holds the ticket.
3. Both sides compute `SAS = blake3::keyed_hash(secret, inviter_id ‖ joiner_id)[..]` → 6 digits, and display it.
4. When the user confirms, the inviter (which must hold an **admin cert** signed by the library root key, decision #18) signs
   `DeviceCert { device: joiner_id, role, name, added_by, hlc }` and sends `LibraryCapability` + the cert.
5. The joiner opens the library and starts syncing. Its role comes from the joiner's request (`--role keeper`)
   and the inviter shows it on the confirm sheet (mockup 5h).

Album invites reuse steps 1–2 but skip the SAS check (they're bearer tickets) and return an album capability instead.

---

## 5. Keeper CLI (`apps/keeper`)

```
shoebox-keeper pair <ticket> [--name NAME] [--storage PATH] [--role keeper|viewer]
shoebox-keeper run                       # foreground daemon (what systemd runs)
shoebox-keeper status [--json]           # mockup 5i
shoebox-keeper album list | keep <name> | drop <name>
shoebox-keeper devices [revoke <id>]     # needs root key or admin cert
shoebox-keeper verify [--repair]         # v1.x (F7.5): re-hash, fetch from peers on mismatch
shoebox-keeper service install|uninstall # systemd unit on Linux, launchd on macOS
```

- Config + state in `$XDG_DATA_HOME/shoebox` (or `--storage`). The endpoint secret goes in a `0600` file (no keychain on headless boxes).
- `run` exposes a local control socket (Unix domain socket) so `status` and the other commands talk to the running daemon
  instead of opening the store twice.
- Logging with `tracing`. `--json` on read commands is for scripting and monitoring.
- Ship static Linux binaries for x86_64 + aarch64 (Pi) built with cross. A Docker image is an easy follow-up.

---

## 6. Tauri 2 shell (`apps/desktop-mobile`)

**Hosting:** `Node` runs in-process in `src-tauri` on the Tokio runtime Tauri provides. One `Node` per app instance.

**Bridge**
- *Commands* (UI → Rust): `library_page`, `photo_detail`, `set_favorite`, `delete`, `import_paths`, `create_album`,
  `invite`, `join`, `pair_start`, `pair_confirm`, `devices`, `health`, `free_up_space`, `settings_get/set`.
- *Events* (Rust → UI): `Change` stream, sync progress, peer presence, pairing SAS, fetch progress per photo.
- *Images:* a custom URI scheme protocol `shoebox://blob/<hash>?kind=thumb` serves blob bytes straight from the store,
  so the webview never receives base64 and the grid scrolls fast.

**UI:** Svelte 5 + Vite (decision #15). Start by porting `design/mockups` components (tiles, lists, sheets, pills) as-is. The
mockup CSS tokens become the design tokens.

**Mobile-specific plugins** (Tauri plugins with Swift / Kotlin parts):
- `photo-library`: enumerate the camera roll, observe changes, read original bytes (PHPhotoLibrary / MediaStore). **No
  off-the-shelf Tauri plugin is assumed. Budget for writing this one.**
- `bg-sync`: register BGProcessingTask (iOS) / WorkManager (Android). It wakes a short-lived `Node::sync_burst(deadline)`
  and posts the "open the app to finish" notification (mockup 5d) when the rules allow.
- `keep-awake`: the toggle in mockup 5e.
- QR scanning: an existing barcode-scanner plugin if one is suitable, otherwise part of `photo-library`'s native side.

**Desktop-specific:** watched folders (`notify` crate), drag-and-drop import, tray icon with sync status.

---

## 7. Milestones

Solo developer, strictly linear (decision #20). Issue IDs are in execution order.

| # | Milestone | Exit criteria (demo on real devices) |
|---|---|---|
| M0 | **Foundations** | Pinned iroh versions; workspace + CI + license check; core model, merge rules, store traits, in-memory backend, conformance suite green. |
| M1 | **Sync core** | Two desktops (via a dev CLI) import, sync metadata, and fetch blobs by role over LAN and relay; copy counts correct; benchmark decides the metadata backend. |
| M2 | **Identity, pairing & keeper** | Signed device list with admin certs; recovery kit; SAS pairing; `shoebox-keeper` runs as a service, syncs every original, and reports status. |
| M3 | **Desktop app** | Tauri desktop app covering every desktop-relevant MVP screen; signed installers for macOS, Windows and Linux. |
| M4 | **Mobile apps** | iOS + Android: camera-roll backup, Phone role, best-effort background sync with honest states; TestFlight / Play internal builds. |
| M5 | **Shared albums** | Create, invite, join, contribute, location prompt, member removal, keeper keep-originals; within limits of 50 members and 50k items. |
| M6 | **Hardening & beta** | Real-network, scale and failure suites green; security review done; closed beta with 10–20 households. |
| M7 | **MVP release** | Docs, store submissions, tagged 1.0-mvp release. |

**Decision gate after M1-15:** if the benchmark favours Loro, insert "Switch default backend to Loro" before M2
and re-estimate. The trait boundary keeps M2+ unaffected otherwise.

## 8. Testing

- **Unit:** `core::merge`, `core::policy` (redundancy, free-up-space safety), and `identity` (signature checks, revocation).
- **Conformance:** one suite, run against every `StoreBackend`, including concurrent-edit scenarios from the merge rules.
- **Multi-node (testkit):** 3–5 in-process nodes with local discovery and no relay. Scripted scenarios: offline edits
  then reconnect, a delete racing an edit, a keeper joining late, revoking a device, freeing space while a holder is offline.
- **Real network:** a nightly job with a keeper on a VPS and a runner behind NAT, to exercise hole-punching and relay fallback.
- **Corpus:** a 10k-photo fixture library (mixed HEIC/JPEG/RAW/video) for import and bench runs.

---

## 9. Risks & open items

| Risk | Mitigation |
|---|---|
| iroh-docs maturity / performance at 100k+ entries | Trait boundary + bench before M3; Loro spike ready behind a feature flag. |
| No Tauri camera-roll plugin; iOS background limits | Budget M4 for native plugin work; keeper is the redundancy backstop; honest UI (5d–5f). |
| HEIC/RAW decode on every platform | Use OS decoders on mobile through the plugin; libheif on desktop/keeper; fall back to "preview generated by the importer" (F2.5). |
| Lost admin devices | If no admin device is left, restore from the recovery kit (root key) to sign a new admin cert. Covered in M2-02 / M2-03. |
| Large initial sync (600 GB to a new keeper) | Resumable, verified streaming is native to iroh-blobs; prioritise thumbnails → previews → originals; show ETA. |

**Resolved before M0:** Svelte 5 (#15), iOS 16 / Android 10 (#16), Apache/MIT libraries + AGPL apps (#17),
delegated admin certs (#18).
