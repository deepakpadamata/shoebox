# iroh-blobs 0.103.1, patched for shoebox

A copy of the crates.io release with one change: the file store's metadata database (redb) takes its
page cache size from `Options::db_cache_size` instead of always using redb's default of 1 GiB.

Why: small blobs (thumbnails, outboards) are stored inside the database, and redb keeps every page it
reads cached up to the cache size. Asking `status()` about an inlined blob reads its data to report the
size, so a device that asks about every thumbnail ends up holding all of them in memory: about 300 MB
for 18,000 thumbnails, heading for 1 GiB as a library grows. `shoebox-node` sets 32 MB.

The change (search for `db_cache_size`):

- `src/store/fs/options.rs`: the `db_cache_size` field and `DEFAULT_DB_CACHE_SIZE` (1 GiB, so the
  default behaves as upstream).
- `src/store/fs.rs`: passes it to the metadata actor.
- `src/store/fs/meta.rs`: opens the database with `redb::Database::builder().set_cache_size(..)`.

Drop this copy once upstream has an equivalent option (not in 0.105.0; worth proposing there) and the
workspace upgrades to that release.
