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
