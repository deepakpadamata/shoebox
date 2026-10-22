---
id: M0-04
title: Core domain model types
milestone: M0 · Foundations
labels: [area:core, type:feature]
estimate_days: 1.5
depends_on: [M0-02]
features: [F2.4]
---

# M0-04 · Core domain model types

## Why

Every crate shares these types. Getting the encoding stable early avoids migrations later.

## Scope

- `PhotoId` (BLAKE3 of the original), `BlobRef {hash,size,mime}`, `BlobKind`, `PhotoRecord`, `ExifSummary`, `AlbumMeta`, `AlbumId`, `SharedItem` (with optional `shared_hash`), `Member`, `MemberRole`, `DeviceCert`, `AdminCert`, `Role` (Keeper/Phone/Viewer), `Holding`, `Hlc`, `Timestamp`.
- Serde + postcard encoding with an explicit version byte per record type.
- `Hlc` implementation (physical ms + logical counter + device id tie-break).

## Acceptance criteria

- [ ] Round-trip tests for every type.
- [ ] Golden-bytes tests lock the v1 encoding.
- [ ] `shoebox-core` has no async runtime or iroh dependency.

---
_Depends on: M0-02 · Estimate: 1.5d · Features: F2.4_
