# Loro fixtures

`index-records.cap` and `index-records.loro` hold a signed library written by the build that added index records
(S1-06): three certified devices, one photo, one `IndexShard` and one `IndexerStatus`. Made by:

```bash
cargo test -p shoebox-store-loro --test index_records -- --include-ignored write_index_records_fixture
```

`index-records-via-0.1.0.loro` is that document after a replica of the build before index records (commit
`f9006ce`, version 0.1.0) imported it, added a photo of its own and exported its snapshot. The old build imported it
without an error, rejected the two new records as undecodable (logged at `debug` only) and kept them in its document.
To make it again, check out that commit in a worktree, add a test that opens the library from `index-records.cap` as
device `KeyPair::from_seed([2; 32])`, imports `index-records.loro`, upserts a photo and writes `export_snapshot()`.

`index_records.rs` `records_kept_by_an_older_replica_are_accepted` checks this build accepts the records from it.
