# Failure scenarios (M6-03)

Real nodes on loopback, driven by `crates/shoebox-store/tests/failures.rs` (plus the older suites
named below). Every scenario asserts two things: **no original is lost** (the device that imported it
still holds it) and **the copy counts shown to the user are truthful** (they never claim a copy that is
not on a disk).

| Scenario | What happens | What the test asserts |
|---|---|---|
| Offline edits on all devices | Three devices edit while partitioned, then reconnect | all converge to the same library (`cluster.rs`, `store-docs/three_nodes.rs`) |
| Delete racing a favorite | One device deletes a photo while another favorites it, partitioned | after healing both show it deleted *and* favorited (independent fields), the original is still on disk until the 30-day window ends, and a restore brings it back everywhere |
| Keeper joining late | A keeper is offline while two phones import, favorite and delete | it converges with everyone, fetches the originals still wanted, the importer lost nothing, every device then reports 2 copies |
| Revocation mid-transfer | An admin revokes a keeper while it is downloading | the other keeper still completes, the importer still holds everything, the copy count is 2 (importer + remaining keeper), never 3 |
| Free up space with holders offline | A phone wants to free originals while other holders are unreachable | nothing is freed on the strength of copies that were not confirmed this week (`freeup.rs`) |
| Disk full on a keeper | The keeper's blob directories refuse new files | it announces only what it really holds (announced copies never exceed originals on disk), keeps retrying, and finishes once there is room; the importer lost nothing |
| Clock skew of ±1 day | One device's clock is a day fast, another a day slow | both devices converge; the clock that is ahead wins conflicts while it is ahead (last-writer-wins is by hybrid time) and edits made after the clocks agree win again; nothing is lost |

## What the scenarios taught us

* Last-writer-wins by hybrid clock means a device whose clock is far ahead wins every conflict until the
  others catch up. That is a trade-off we accept for a personal library (a conflict needs two people
  editing the same field of the same photo within a day); the alternative needs trusted time.
* Libraries only sync between *certified* devices: two nodes that merely share a capability, with no
  device list, do not exchange records. The tests therefore build a proper identity first, as the app does.
* A failed write leaves the blob unannounced, because holdings are published only after the blob is
  verified in the store (`LibraryNode::publish_holdings` after `fetch_from_holders`).
