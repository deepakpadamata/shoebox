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

## Records

| What | Encoding |
|---|---|
| library records (photos, albums, certificates, holdings, settings) | `[version byte] ++ postcard(body)`, version 1 (`codec.rs`, `entry_codec.rs`) |
| Loro capability payload | version byte 1 (`CAPABILITY_VERSION`); `[2, identity id]` after an epoch change |

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
| `shared-albums.json` | shared-album registry (`0600`: holds album capabilities) |
| `secrets/` | file keystore (dev, tests); real builds use the OS keystore |
| `posters/` | device-local video poster frames |
| `backups/` | copies taken before a migration |
| `node/` | the node's data dir, below |

Node data dir (the keeper storage dir is one, plus `keeper.json`, `keeper.lock`, `control.sock`):

| Path | What |
|---|---|
| `config.toml` | node settings, watched folders, pinned originals |
| `keys/` | endpoint key file (when no OS keystore) |
| `blobs/` | iroh-blobs store; tags `lib/<hash>`, `pin/<hash>`, `album/<album>/<hash>`, `place/<hash>` (an original referenced in place, a phone's gallery file) |
| `store/docs/` | iroh-docs store (shared albums) |
| `loro/<epoch>/` | the library: `snapshot.loro`, `updates.log` |
| `holder-confirmations.json` | who confirmed holding which original (free-up) |
| `epoch-notice.bin` | the latest epoch notice, postcard `EpochNotice` (layout 2; layout 1 had `rotation-notice.bin`) |
| `admins-seen.bin` | admins this device has seen, for new-admin alerts |

Layout history: 0 = no marker, 1 = marker (M6-05), 2 = 0.1.0 (epoch notice file).
