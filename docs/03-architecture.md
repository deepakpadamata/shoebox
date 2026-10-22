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

- **Cloud backup**: a zero-knowledge keeper. Devices upload blobs encrypted with a library key it
  never sees. It counts toward copy count, but it can't serve previews or the web view.
- **Gateway "relay view"**: renders an album on the web for people without the app. Because the backup is
  zero-knowledge, the gateway needs album read keys from a share link (e.g. in the URL fragment), or
  must proxy from an online member device. Still open.

## Security notes

- In transit: iroh connections are QUIC/TLS authenticated by endpoint keys; relays can't read traffic.
- At rest: OS-level encryption by default; blobs are encrypted client-side for any store we don't trust (reciprocal backup F7.6, hosted backup F7.7).
- Capabilities are bearer tokens — invite tickets should be single-use or expiring where possible.
- Originals (with location) are shared by default; stripping is opt-in per share via a prompt (F8.3).
