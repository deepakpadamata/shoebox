---
id: M1-16
title: Switch the personal-library backend to Loro
milestone: M1 · Sync core
labels: [area:store, type:feature]
estimate_days: 16
depends_on: [M1-15]
features: []
---

# M1-16 · Switch the personal-library backend to Loro

## Why

M1-15 showed iroh-docs, used as one entry + one blob per record, does not scale to the 100k-photo target (initial sync 29 s at 60k,
fails at 100k) while the Loro spike syncs 100k in 0.8 s and 500k in 6.8 s at a fifth of the disk. See `docs/notes/store-benchmark.md`.
Shared albums stay on iroh-docs (capabilities, bounded size).

## Scope

- Production `LibraryStore` on Loro (promote `shoebox-store-loro` from spike): iroh transport (ALPN, version-vector exchange, retries,
  backoff, convergence check), update announcements via the presence topic, snapshot + incremental updates persisted as blobs with
  compaction (shallow snapshots), crash-safe storage.
- Authenticity: verify device signatures on imported entries (needs M2-01's signed device list; until then trust the capability holder).
- Port `LibraryNode` (sync manager, presence, fetcher, GC hooks) and the testkit scenarios to the new backend; keep iroh-docs for albums.
- Incremental rescan from Loro change events instead of O(n) scans; memory work on `LibraryState` (intern strings, publish holdings for
  originals only) — measured on a phone-class device.
- Conformance suite (all scenarios except shared albums) and the 3-node partition/heal scenario pass; re-run `store-bench` at 100k/500k.

## Acceptance criteria

- [ ] Conformance suite (library scenarios) and the testkit 3-node scenario pass on the Loro backend.
- [ ] 100k photos: initial sync < 10 s over loopback, replica on disk < 100 MB; 500k completes.
- [ ] A read-only viewer cannot publish entries that other devices accept (signature check).
- [ ] Re-estimate of M2–M7 recorded in `planning/README.md`.

## Notes

Review the decision in `docs/notes/store-benchmark.md` before starting. Alternative if staying on iroh-docs: batch records into per-month entries.

---
_Depends on: M1-15 · Estimate: 16d · Features: —_
