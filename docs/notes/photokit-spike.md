# PhotoKit spike (P1-01, 2026-11-03)

Two questions from [P1-01](../../planning/issues/P1-01-spike-photokit-on-this-mac.md). The first is answered. The
second could not be measured on this Mac, so it moves to later issues.

1. **Can a phone serve an original it keeps no copy of (D1)?** Yes. iroh-blobs can hold a peer's request while the
   original is exported, then serve it. A test proves it.
2. **How does PhotoKit behave on a real library?** Not measured. This Mac's Photos library is empty, and the owner
   chose not to fill it with generated photos. The probe is written and its access request works. The numbers move
   to P1-03 (Mac) and P1-10 (iPhone).

---

## 1 · Serving on request (D1 settled: the first form, no outbox)

**What iroh-blobs offers.** iroh-blobs 0.103.1 (all paths below are in the iroh-blobs source) can **intercept**
requests:
- Each GET is sent to the application as `GetRequestReceived` when the event mask has
  `get: RequestMode::InterceptLog` (`provider/events.rs` 49-66, 475-546).
- The provider waits for an answer before touching the store: `handle_get` in `provider.rs` 472-481 runs before
  `export_bao`.
- The handler is async, so it can take as long as an export takes.
- The requester has no get timeout of its own. The connection's keep-alive (one second) keeps a held request open.

Without interception, a hash the store does not have gets an empty answer, not an error (`store/fs.rs` 1204-1209).
The requester's download then fails. `fetch_now` does not retry, and the background fetcher retries after 10 s.

**The test.**
[`crates/shoebox-node/tests/serve_on_request.rs`](../../crates/shoebox-node/tests/serve_on_request.rs) passed 5 of
5 runs (about 1 s each).
- **Holder:** a store with only the hash. On a GET it:
  - exports the bytes to a temporary file (500 ms, standing in for PhotoKit);
  - imports the file with `ImportMode::TryReference` under a temporary tag;
  - checks the hash, then lets the request go on;
  - at `Completed` or `Aborted`, drops the tag and deletes the file.
- **Requester:** an unchanged Shoebox node using its normal download path.
- **The test asserts:**
  - the holder had no copy before the request;
  - the bytes arrived verified;
  - garbage collection removed the holder's entry;
  - no temporary file was left behind.

**The fallbacks are not needed.**
- **Outbox:** not needed, because interception works.
- **Keeping only the outboard (the hash tree) with data supplied lazily:** this would mean writing a new store actor,
  and it saves only a re-hash per serve.

**What P1-07 has to handle** (Shoebox today uses `NotifyLog`, at `transfers.rs` 180-188 and `node.rs` 575):
- **Every GET waits on the handler**, including docs, previews and thumbnails. A hash already in the store needs a
  fast path that answers at once. Every request must be answered: a handler that never answers leaves the request
  hanging, so give it a timeout.
- **One export per hash, shared.** A split download sends many piece requests to up to 4 holders (`node.rs` 37, 41),
  and several peers can ask at once. The temporary tag is counted per request.
- **Don't trust the store after a deletion.** Between deleting the temporary file and garbage collection, `has()` is
  still true. The handler tracks what it has exported itself.
- **The phone must count as a holder:**
  - The free-up check (`HasProtocol`, `freeup.rs` 61-72) and `holds_original` in the fetcher (`fetcher.rs` 214)
    look at the store, so they would treat the phone as holding nothing.
  - On-request originals need their own record (as `place/` tags do on Android). Otherwise copy counts are too low,
    and the phone tries to fetch its own photos.
- **The exported bytes must be the ones that were hashed.** The same resource is chosen as at hashing time (D2), and
  a different hash is refused, not served.
- **The app must be running.** A suspended iOS app answers nothing: peers fetch while it is open or in a background
  task (M4-08), as on Android.
- **iCloud-only originals are not claimed.** Exporting them would need the network.
- **Free space.** The phone needs room for the photo while serving it. The space comes back when the file is
  deleted.

---

## 2 · PhotoKit on this Mac (not measured)

**The probe.** [`tools/photokit-probe/`](../../tools/photokit-probe/) builds a signed app bundle with a usage string
and runs one command; it reads only. The commands:
- `enumerate`: the fetch and the walk over every asset, and the cost of listing resources;
- `resources`: which resource types appear, edits, Live Photos;
- `local`: which originals are on this device, checked two ways (the `locallyAvailable` key, and a first-chunk read
  without network);
- `stream`: streams originals through SHA-256 and compares with an exported file, giving throughput and peak memory;
- `token` / `changes`: persistent change tokens.

**What was learned:**
- **The access request works for an unbundled tool,** if it is launched as an app bundle with
  `NSPhotoLibraryUsageDescription` through `open`. The system asked on the probe's behalf, and the answer was
  "granted".
- **Each rebuild needs access granted again.** Ad hoc signing makes every rebuild a new app to macOS, so the grant is
  lost; this Mac has no signing identity. For P1-04, sign debug builds with one stable identity, or expect a prompt
  after every build.
- **The library is empty,** so PhotoKit returned 0 assets and 0 albums. The library had been opened, and no filter
  was applied.

**Still to measure:**

| Question | Where now |
|---|---|
| Time to enumerate 20k assets (M4-03 target < 10 s); cost of listing resources per asset | P1-03 on a seeded or real Mac library; P1-10 on an iPhone |
| Stream-hash throughput and memory on a large video; stream hash = exported file | P1-03 (`stream`) |
| Telling iCloud-only originals apart without downloading; whether `locallyAvailable` agrees with a network-off read | P1-03 needs an iCloud library with Optimize Storage; P1-10 |
| What each change (add, edit, favourite, delete) reports through `fetchPersistentChanges`, and whether the token survives a restart | P1-03 (`token` / `changes`) |
| Resource types of edited photos and of Live Photos taken on an iPhone | P1-08, P1-10 |

The probe answers each of these once it is pointed at a library with photos in it: run
`tools/photokit-probe/run.sh auth`, then the command.
