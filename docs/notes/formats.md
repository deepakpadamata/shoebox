# Formats and protocol versions

Everything one Shoebox build says to another, or leaves on disk for a later build, with its version. From **0.1.0**
on these are promises: see "Changing a format" in [`CONTRIBUTING.md`](../../CONTRIBUTING.md).

`crates/shoebox-node/tests/formats.rs` reads every `shoebox/<name>/<version>` string in the source and fails if this
page lists a different set, so a renamed protocol or a new signing domain cannot slip in unnoticed. It also checks the
on-disk layout number below against `shoebox_node::format::CURRENT_FORMAT`.

## Wire protocols (ALPN)

A connection opens one of these by name; a peer that does not serve a name refuses it cleanly
(`tests/compat.rs`).

| Name | What | Who may open it |
|---|---|---|
| `shoebox/pair/0` | pairing a new device (`pairing.rs`) | anyone with a ticket |
| `shoebox/recover/0` | rejoining with the recovery phrase (`recover.rs`) | anyone; needs the root key |
| `shoebox/album/0` | shared-album invitations and moves (`shoebox-store` `shared.rs`) | anyone; checked per message |
| `shoebox/rotate/0` | epoch notices: a postcard `EpochNotice` (revocation or root handover) (`rotation.rs`) | listed devices |
| `shoebox/loro/0` | library sync (`shoebox-store-loro` `net.rs`) | listed devices |
| `shoebox/has/0` | "do you hold these blobs?" before freeing space (`freeup.rs`) | listed devices |
| `shoebox/ping/0` | 64-byte echo for latency (`node.rs`) | listed devices |
| `shoebox/doc-digest/0` | digest of an iroh-docs namespace (`doc_digest.rs`) | anyone who knows the namespace |
| `shoebox/webview/0` | a browser reading a Live web link: manifest and blob ranges of one share, ciphertext only; wire format in `web_live.rs` (W1-08) | anyone who knows the share id (open to non-members in the connection gate) |
| iroh-blobs, iroh-docs, iroh-gossip | their own ALPNs, versions as pinned in [`iroh-versions.md`](iroh-versions.md) | see the connection gate (A-079) |

## Gossip topics

| Name | What |
|---|---|
| `shoebox/presence/2` | presence topic: blake3 of the name and the library's identity id (`presence.rs`) |

## Signing and hashing domains

Each signature or keyed hash starts with its domain, so one can never be replayed as another.

| Domain | Signs / hashes |
|---|---|
| `shoebox/device-cert/v1` | device certificates |
| `shoebox/admin-cert/v1` | admin certificates |
| `shoebox/rotation/v1` | revocation notices (`RotationNotice`) |
| `shoebox/root-handover/v1` | root handovers (`RootHandover`), by the old and the new root |
| `shoebox/presence-msg/1` | presence messages |
| `shoebox/recover/v1` | the recovery challenge answer |
| `shoebox/pair-proof/0` | the pairing proof (keyed BLAKE3) |
| `shoebox/pair-sas/0` | the six-digit pairing code (keyed BLAKE3) |
| `shoebox/pair-ticket-id/0` | the pairing ticket's lookup id |
| `shoebox/loro-entry/v1` | signed records in the Loro library |
| `shoebox/cloud-secret/v1` | the library's cloud secret, derived from the root seed (W1-02) |
| `shoebox/cloud-blob-key/v1` | a blob's cloud key, from the cloud secret and the plaintext hash |
| `shoebox/cloud-nonce/v1` | a piece's nonce (keyed BLAKE3 by the blob key, piece number, last flag) |
| `shoebox/share-manifest-key/v1` | the key that opens a share manifest, derived from the link's share key (W1-03) |
| `shoebox/cloud-request/v1` | a signed request to Shoebox Cloud (`CloudAuth`): method, path and query, time, body hash (W1-04) |
| `shoebox/share-signer/v1` | the key a share manifest is signed with, derived from the share key (W1-05) |
| `shoebox/import-album/v1` | an imported album's id: BLAKE3 of the source kind and the album's title, so importing again adds to the same album (`import_archive.rs`, I1-07) |
| `shoebox/import-run/v1` | an archive import's run id: BLAKE3 of the parts' names and sizes, naming its journal (`import_archive.rs`, I1-04) |
| `SBXS` (the magic) | a sealed share manifest's signature covers every byte from the magic on |

Sealed cloud blobs are 4 MiB pieces of XChaCha20-Poly1305 (16-byte tags, no header); locked by golden bytes in
`crates/shoebox-core/tests/cloud.rs`. Sealed share manifests are laid out in the header of `crates/shoebox-core/src/share.rs`
and locked in `tests/share.rs`.

## Records

| What | Encoding |
|---|---|
| library records (photos, albums, certificates, holdings, settings) | `[version byte] ++ postcard(body)`, version 1 (`codec.rs`, `entry_codec.rs`) |
| Loro capability payload | version byte 1 (`CAPABILITY_VERSION`); `[2, identity id]` after an epoch change |
| people records (S1-18) | `persons/<id hex>` → `StateEntry::Person` (name, hidden, merged into: last writer wins per field) and `facelabels/<photo hex>/<face key hex>` → `StateEntry::FaceLabel` (`is` last writer wins, `is_not` add-wins). Appended after the index records; the face key is the box on a 1/64 grid of the photo (4 bytes) |
| index records (S1-06) | `index/<blob hex>` → `StateEntry::IndexShard` (immutable; any member but a Viewer) and `indexers/<device hex>` → `StateEntry::IndexerStatus` (only that device). Both **appended** to `StateEntry`, so 0.1.0 rejects them as undecodable (at `debug`), keeps them in its document and passes them on (`shoebox-store-loro/tests/index_records.rs`) |
| blob kinds | `BlobKind::Index` appended (holdings of index shards) and `BlobKind::Motion` (holdings of a Live Photo's video; a video is never counted as a copy of the original) |
| motion records (F2.10) | `motion/<photo hex>` → `StateEntry::Motion` (`MotionRecord`: photo, video `BlobRef`, `duration_ms`, `source` Paired or Embedded, author, HLC). One per photo, the later (HLC, author) wins; written by any member but a Viewer, for a photo that exists. Appended to `StateEntry` on 2026-10-31, before the 0.1.0 release; a build that does not know it keeps it and passes it on, and shows the still. The video is a blob of its own (`lib/<hash>`): the file next to the picture (Apple's `.MOV`, Google's `.MP4`) or the MP4 cut out of a Samsung or Google motion photo, whose original stays the whole file |
| index shard blobs (S1-06) | `"SBIX"`, version byte 1 (`shoebox_search::shard::VERSION`), kind byte (`ShardKind`: 0 vec8, 1 vec1, 2 text, 3 faces), `ModelId` (32), dims u16, count u32, `replaces` (u16 n + n × 32), sorted photo ids, rows; layout in `crates/shoebox-search/src/shard.rs`. At most 1,024 photos. Immutable and content addressed: a new version is a new kind of blob, never a rewrite |
| shared-album records (meta, items, members, events, people) | `[version byte] ++ postcard(body)`, version 1. `people/<device>` (`AlbumEntry::Person`, which person a device belongs to) was appended to the enum on 2026-10-27, before the 0.1.0 release: builds from before that skip it |

Library settings with a meaning (LWW per key, opaque bytes):

| Key | Value |
|---|---|
| `redundancy.target` | one byte: copies to keep |
| `recovery_kit_saved` | one byte |
| `search.people` | one byte: 0 = people (faces) off for the library; absent or anything else = on (S1-17) |
| `people.snapshot` | postcard `BlobRef` of the newest people grouping snapshot (postcard `shoebox_search::people::Snapshot`, version 1), kept as search data (S1-18) |
| `people.me` | 16 bytes: the `PersonId` that is "me" (S1-18) |
| `profile.name` | UTF-8: the person's name as people they share with see it (A-097) |
| `shared.albums` | JSON list of `AlbumRef` (`shoebox_store::shared`): the shared albums the person is in, with capability, inviter, role, `cap_ms`, `joined_ms`, `left_ms`, `from_album`, `location` (A-097) |

## Text people copy

| Prefix | What |
|---|---|
| `shoebox-pair1…` | pairing ticket (QR) |
| `shoebox-peer1…` | a device's link, for recovery (no secret) |
| `shoebox-library1…` | developer ticket (`shoebox-dev` only) |
| 24 BIP39 words | the recovery phrase: the root key's seed |

## On disk: layout 2 (0.1.0)

`format.json` in the app data dir and in the keeper storage dir holds the layout number (`shoebox_node::format`).
Opening older data copies the small files aside and migrates; newer data is refused untouched.

App data dir:

| Path | What |
|---|---|
| `format.json` | layout marker |
| `app.json` | device name, role, library ticket, relay, unacknowledged security alerts |
| `app.lock` | one app per data dir |
| `backup.json` | camera-roll backup state (phones): policy, change token, waiting assets, `tracked` (asset id → photo, path, modified time), `moving`, `held_deletions`; an older file with only `imported` is read and replaced |
| `shared-albums.json` | shared-album registry (`0600`: holds album capabilities); per album also `own` (the person's other devices in it) and `cap_ms` (when the capability was set), both optional |
| `secrets/` | file keystore (dev, tests); real builds use the OS keystore |
| `posters/` | device-local video poster frames |
| `backups/` | copies taken before a migration |
| `node/` | the node's data dir, below |

Node data dir (the keeper storage dir is one, plus `keeper.json`, `keeper.lock`, `control.sock`):

| Path | What |
|---|---|
| `config.toml` | node settings, watched folders, pinned originals, keep rules (`keep_recent_days`, `keep_favorites`); a role of `Phone` from earlier builds reads as `Companion` |
| `keys/` | endpoint key file (when no OS keystore) |
| `blobs/` | iroh-blobs store; tags `lib/<hash>`, `pin/<hash>`, `album/<album>/<hash>`, `place/<hash>` (an original referenced in place, a phone's gallery file) |
| `store/docs/` | iroh-docs store (shared albums) |
| `loro/<epoch>/` | the library: `snapshot.loro` (Loro bytes any `import` reads: a full snapshot from builds before M4-11, all updates since; both open), `updates.log` |
| `holder-confirmations.json` | who confirmed holding which original (free-up) |
| `epoch-notice.bin` | the latest epoch notice, postcard `EpochNotice` (layout 2; layout 1 had `rotation-notice.bin`) |
| `admins-seen.bin` | admins this device has seen, for new-admin alerts |
| `search/vec-<model>.bin` | the picture vectors of one model (S1-12): `"SBVX"`, version 1, model, dims, count, ids, scales, binary codes, then the 8-bit vectors. **A derived cache**, rebuilt from the shards |
| `search/indexer-since` | when this device first indexed (milliseconds), where the 48-hour takeover counts from (S1-11) |
| `search.json` (app data folder) | this device's search settings (`{"index": bool}`) |
| blob tag `idx/<hash>` | search data this device keeps: index shards, the people snapshot, model files (S1-07) |
| `search/facts.idx` | the search facts index (S1-03): `"SBFX"`, version byte (`FACTS_VERSION`, 2: facts gained `live`), BLAKE3 of the payload, postcard snapshot. **A derived cache:** deleted, damaged or of another version, it is rebuilt from the library, so it needs no migration and no layout bump |

Layout history: 0 = no marker, 1 = marker (M6-05), 2 = 0.1.0 (epoch notice file).
