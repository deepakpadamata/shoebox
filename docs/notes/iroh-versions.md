# iroh versions & API notes (M0-01)

Verified 2026-10-22 against crates.io and by running `examples/hello-sync`
(two in-process endpoints on loopback: one blob transfer + one iroh-docs entry sync).

## Pinned versions

| Crate | Pin | Notes |
|---|---|---|
| `iroh` | `=1.4.1` | 1.x line is stable. Endpoint/Router/ProtocolHandler as assumed. |
| `iroh-blobs` | `=0.103.1` | Requires `iroh ^1`. `MemStore` / `FsStore`, `BlobsProtocol`, downloader. |
| `iroh-gossip` | `=0.101.0` | Requires `iroh ^1`. |
| `iroh-docs` | `=0.101.0` | **Maintained** (released 2026-06-15), requires `iroh ^1`, `iroh-blobs ^0.103`, `iroh-gossip ^0.101`. Builds and syncs against the pins above. |
| `iroh-mdns-address-lookup` | `=0.6.1` | LAN discovery is a **separate crate** in iroh 1.x (not a feature of `iroh`). Resolves against the pins. |

Rationale: take the latest release of each crate whose declared `iroh` requirement is satisfied by one
`iroh` version, then pin exactly (`=`) so a `cargo update` never silently moves the stack. Bump all of them
together and re-run `cargo run -p hello-sync`.

**Verdict: no blocker.** iroh-docs is viable, so M1-05 stays on iroh-docs and M1-14 (Loro) stays a spike.

## API names (corrections to the plan)

The names in `05-prototype-plan.md` were from memory; actual 1.x names:

- `Endpoint::bind(presets::N0)` (relay + n0 discovery) or `Endpoint::builder(presets::Minimal)...bind()` (nothing). Presets: `Empty`, `Minimal`, `N0`, `N0DisableRelay`.
- `endpoint.addr()` → `EndpointAddr`; `endpoint.id()` → `EndpointId`.
- Address lookup (formerly "discovery"): `iroh::address_lookup::memory::MemoryLookup` for out-of-band addresses
  (`add_endpoint_info(endpoint.addr())`), `Builder::address_lookup(..)` to install it.
- Custom protocols: implement `iroh::protocol::ProtocolHandler` (`async fn accept(&self, Connection)`),
  register with `Router::builder(endpoint).accept(ALPN, handler).spawn()`. ALPNs are plain byte strings
  so `shoebox/pair/0` works as planned.
- Blobs: `iroh_blobs::store::mem::MemStore` / `store::fs::FsStore`; `store.blobs().add_bytes(..)` / `add_path(..)`;
  `BlobsProtocol::new(&store, None)`; fetch with `store.downloader(&endpoint).download(hash, Some(endpoint_id))`;
  read with `store.blobs().get_bytes(hash)`.
- Docs: `Docs::memory()` / `Docs::persistent(path)` `.spawn(endpoint, blobs_store, gossip)`; needs blobs + gossip
  on the same router (`ALPN`s `iroh_blobs::ALPN`, `iroh_gossip::ALPN`, `iroh_docs::ALPN`).
  `docs.create()`, `doc.set_bytes(author, key, value)`, `doc.share(ShareMode, AddrInfoOptions)` → `DocTicket`,
  `docs.import_and_subscribe(ticket)`, `LiveEvent::{InsertLocal, InsertRemote, ContentReady, NeighborUp, ..}`.
  Entry **values are blob hashes**; content moves over iroh-blobs (auto-downloaded by default policy).
- Tickets: `iroh-tickets` crate (`BlobTicket`, `DocTicket`).

## Gaps / watch-list

- `cargo tree -d` shows two `redb` majors (3.1.3 via iroh-blobs, 4.4.0 via iroh-docs). Harmless, bigger binaries.
- The spike used loopback + `MemoryLookup`, **not** a real relay or NAT. Real hole-punching/relay is exercised by M6-01.
- mDNS was verified to resolve and compile only; behaviour on a real LAN is M1-09's acceptance.
- `iroh-docs` timestamps are per-author last-write; domain LWW stays in `core::merge` as planned.
