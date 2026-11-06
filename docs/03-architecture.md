# 03 · Architecture sketch (how features map onto iroh)

This is a first sketch to sanity-check that the feature set is buildable, not a final design.
Verify crate names/APIs against current iroh releases before building — the ecosystem moves
quickly (e.g. `NodeId` → `EndpointId` naming).

## Building blocks

| iroh piece | What we use it for |
|---|---|
| **`iroh` Endpoint** | One per device. Its public key is the device ID. QUIC connections dialed by key; hole-punching with relay fallback. Local (mDNS) discovery for LAN sync (F4.7). |
| **`iroh-blobs`** | All photo/video bytes: originals, previews, thumbnails. BLAKE3 content addressing gives dedupe (F2.4), verified streaming + range requests (F2.9, F3.2), cheap integrity checks (F7.5). |
| **`LibraryStore` interface** → `iroh-docs` first | Replicated metadata: the library index, albums, device registry. Prototype on iroh-docs (multi-writer, range-based set reconciliation); benchmark a Loro/Automerge CRDT over gossip + blobs at 100k+ entries before committing (decision #7). |
| **`iroh-gossip`** | Presence ("MacBook is online"), live "new photo" notifications, profile updates within a library or shared album swarm. |
| **Tickets** | Pairing (F4.1), album invites (F6.1), one-off blob sends (F6.6). Encoded as links + QR. |

## Data model

```
Library (one per person)            ── replicated namespace, written by my devices only
├── photos/<photoId>                → { originalHash, previewHash, thumbHash, takenAt,
│                                       mime, size, exif, importedBy, favorite, deletedAt? }
├── albums/<albumId>                → { name, cover, items: [photoId…] }   (personal albums)
├── devices/<endpointId>            → { name, role, addedAt, revokedAt? }
└── holdings/<endpointId>/<hash>    → { kind: original|preview|thumb, since }   ← copy-count (F2.6, F3.4, F7.1)

Shared album (one namespace per album) ── written by members with "can add"
├── meta                            → { name, cover, createdBy }
├── items/<photoId>                 → { originalHash, sharedHash?, previewHash, thumbHash, addedBy, addedAt, takenAt }
│                                       sharedHash set only if the user chose to strip location (decision #8)
├── members/<memberKey>             → { displayName, role }
└── events/<ts>-<memberKey>         → activity feed (F6.4)
```

- `photoId` = BLAKE3 hash of the original. Two devices importing the same file converge on one entry.
- Blobs are fetched lazily according to the device's storage role (F4.3):
  - **Keeper**: fetch every original.
  - **Phone**: thumbs + previews for everything, originals only for own captures (until redundancy met) and pinned items.
  - **Viewer**: thumbs + previews only.
- Shared albums: keepers fetch other members' originals only if that album's "Keep originals" is on (F6.9).
- **Holdings** entries let any device compute "how many copies of this original exist" without
  contacting everyone. Freeing space (F7.3) requires the count to be ≥ target *and* at least one holder to have confirmed recently.

## Identity (decision #9)

- Each device has its own iroh endpoint key. Keys are never shared between devices.
- The **library root key** lives in the recovery kit (and, optionally, on the first device). It signs
  `devices/` entries: additions and revocations. Peers only accept library writes from devices in the signed list.
- Recovery = restore the root key on a new device → sign it into the list → sync from any surviving holder.
- Open: whether day-to-day pairing needs the root key on hand, or whether existing devices can co-sign
  (e.g. a delegated "admin" cert per device).

## Flows

### Pairing a device (F4.1)
1. Device A (already in library) shows QR: ticket = { A's endpoint addr, one-time pairing secret }.
2. Device B scans, dials A over iroh, both derive and display a 6-digit SAS code; user confirms.
   (Headless keeper: `shoebox-keeper pair <ticket>` prints the SAS code in the terminal.)
3. A sends B the library namespace capability (write) and adds a root-signed `devices/<B>` entry.
4. B starts syncing metadata, then thumbnails, then according to its role.

### Sharing an album (F6.1–F6.3)
1. Create a new namespace for the album; copy referenced items' metadata into it. If any items carry
   location, prompt: share as-is (default) or strip. Stripped items get a derived blob and `sharedHash`.
2. Invite ticket carries read or write capability for *that namespace only*, plus a hint of peer addresses.
3. Recipient joins, syncs metadata, fetches blobs from any member who has them (blobs are content-addressed, so source doesn't matter).
4. Removing a member: create a fresh namespace, migrate, re-invite remaining members (true revocation in a replicated system requires key rotation).

### Fetching an original (F3.2)
Ask holders (from `holdings/`) in order: LAN peers → direct-connected peers → relayed peers. Stream with
verification; show source device in UI.

## Search (epic S1)

Search runs entirely on the device that searches; only the one-time indexing is shared.

- **Facts and places** (`shoebox-search`, `facts.idx`) are derived on each device from the photo records it already
  has: dates, cameras, albums, and place names from the bundled gazetteer. Nothing is synced for them.
- **Index shards** are immutable blobs (`SBIX`, up to 1,024 photos, one model and kind: `vec8`, `vec1`, `text`,
  `faces`), recorded in the library as small `IndexShard` records (`index/<blob>`). Every device holds every live
  shard. A shard replaces older ones by listing them inside its blob; per photo, the shard with the lowest hash wins,
  so two devices indexing the same photo is harmless.
- **Indexing** is shared without claims: each device does its own imports; the backlog goes to always-on indexers by
  weighted rendezvous hashing over their `IndexerStatus` (`indexers/<device>`), or, when no always-on device is
  indexing, to every device that is (`share_candidates`, A-124), so a phone on its own or phones only share it too;
  anyone takes over a photo still unindexed after 48 hours. A keeper indexes always, a desktop on power when idle, a
  phone while charging.
- **Models**: OpenCLIP ViT-B/32 at 8 bits (the text side in every build, the image side on indexing devices; phones
  fetch it as a blob by its compiled-in hash), YuNet and SFace for faces, the system's OCR (Apple Vision on macOS),
  all run with `tract` (`shoebox-ml`). See `docs/notes/search-models.md`.
- **On each device**: the query is parsed into filters and words; filters select photos from `facts.idx`; words rank
  by keywords (labels, text, places, albums, names) and by meaning (the text model's vector against the 8-bit
  picture vectors, streamed from `search/vec-*.bin`), fused by reciprocal rank.
- **People**: `Person` and `FaceLabel` records (`persons/…`, `facelabels/…`) hold what people said; one device groups
  faces into a snapshot blob (`settings/people.snapshot`); every device works out who each face is from labels, the
  snapshot and the face shards, the same way.

## Platforms

- **Core** in Rust (iroh is Rust-native): library model, sync engine, blob store, import pipeline.
- **UI**: **Tauri 2 everywhere** (macOS, Windows, Linux, iOS, Android) with the Rust core in-process and
  a single shared web UI (decision #11). Desktop and mobile are built in parallel (decision #14).
- **Headless keeper** (F4.6, MVP) = the core + a CLI/daemon. Same binary crate, no Tauri.
- **Mobile background sync** is best-effort (decision #10): opportunistic OS background tasks
  (iOS BGTaskScheduler via a Tauri plugin, Android WorkManager), with the keeper as the backstop.
  Validate early what Tauri 2 can reach; a small native plugin is likely needed.
- **Relays**: n0 public relays for MVP; our own once v2 hosted services exist (decision #12).

## Hosted services (v2, optional)

One service, **Shoebox Cloud** (epic W1), does both jobs; it holds ciphertext only.

- **Cloud keeper**: a zero-knowledge keeper. Devices upload blobs encrypted per blob (`k_b` derived from a
  library cloud secret in the recovery kit). It counts toward copy count as an "encrypted copy".
- **Web links**: a share is an encrypted manifest listing `{cid, k_b}` for the shared photos. The link's
  `#fragment` carries the share key, so the server never sees it, and a static viewer page decrypts in
  the browser. Works with every device off. A free **Live** mode has the browser fetch the same encrypted
  objects from the sharer's online devices over iroh (`shoebox/webview/0`).

## Security notes

- In transit: iroh connections are QUIC/TLS authenticated by endpoint keys; relays can't read traffic.
- At rest: OS-level encryption by default; blobs are encrypted client-side for any store we don't trust (reciprocal backup F7.6, hosted backup F7.7).
- Capabilities are bearer tokens — invite tickets should be single-use or expiring where possible.
- Originals (with location) are shared by default; stripping is opt-in per share via a prompt (F8.3).
