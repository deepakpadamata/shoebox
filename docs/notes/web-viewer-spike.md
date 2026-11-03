# Web viewer spike: crypto and iroh in the browser (W1-01)

Measured 2026-11-03 on a Mac (Apple silicon). The browser was the Chromium (152) inside the Claude desktop app's
browser pane, which was **hidden** while it ran, so its timings are a little unreliable. **Not measured:** Safari,
Firefox, and any phone. Those are open items, not assumed fine.

## Verdict

| Question | Answer |
|---|---|
| Crypto path for W1-06 | **WebAssembly build of `shoebox-core`** (`crates/shoebox-web`): 170 KB, 75 KB gzipped, one implementation of the format |
| Can the page reach a device through a relay? | **Yes.** A page in Chromium fetched a 300 KB blob from a native process on the Mac through the n0 relay, byte for byte: 4.6 s cold (bind, address lookup, relay, transfer) |
| Live mode (W1-08) | **Go**, with the bundle cost below and the conditions at the end |

## Crypto: 5 MB blob (two 4 MiB pieces), decrypt and check its id

| Path | Node 26 (V8) | Chromium 152 in the app's pane |
|---|---|---|
| **shoebox-core in WebAssembly**, decrypt + BLAKE3 id | 17 ms | 18 ms |
| shoebox-core in WebAssembly, decrypt only | 13 ms | 26 ms (first run, includes warm-up) |
| `@noble/ciphers` + `@noble/hashes` in JavaScript, decrypt + BLAKE3 | 75 ms | 399 ms |
| `@noble/ciphers` decrypt only | 16 ms | 38 ms |
| WebCrypto AES-GCM decrypt only (reference: **not our format**) | 0.8 ms | 520 ms (odd; the hidden pane) |

- WebCrypto has no ChaCha20-Poly1305, so it cannot open our blobs. AES-GCM would be hardware-fast in a normal
  browser, but switching the format would give up the one-implementation property; not worth it at 380 MB/s.
- JavaScript BLAKE3 is the slow part of the JS path, and the page needs it to check a blob against its id.
- WebAssembly decrypts at about 300 MB/s, far more than a connection delivers, so crypto is never the bottleneck.

## iroh in a browser

- `iroh =1.4.1` builds for `wasm32-unknown-unknown` with `default-features = false, features = ["tls-ring"]`
  (the N0 preset, with its relay and address lookup, needs a TLS provider). `ring` compiles C for wasm, so the build
  needs a clang with the wasm target: **Apple's does not, Homebrew's `llvm` does** (`CC_wasm32_unknown_unknown`,
  `AR_wasm32_unknown_unknown`). Release CI needs the same.
- Size without `wasm-opt`: **5.2 MB, 1.56 MB gzipped**. Too big for the page itself: it loads only when a Live link is
  opened, and the Cloud path (the default) never loads it. `wasm-opt -Oz` was not available; expect some saving.
- A browser reaches a device only through a relay (no UDP). The device needs nothing special: `Endpoint` with the N0
  preset and the ALPN. The spike used a throwaway ALPN, not `shoebox/webview/0`, and iroh-blobs was **not** used in
  the browser: the page speaks a request/response protocol of its own (W1-08).

## Not spiked, and what W1-06 does instead

- **Streaming a 200 MB video** by pieces (`decrypt_piece` is built and tested natively): not tried in a browser.
  W1-06 starts with: videos up to 100 MB are fetched, opened and played from a Blob URL; larger ones download.
  Seeking through pieces (MediaSource, or a service worker) comes if the first version proves too slow.
- **A 1 GB zip** built in the browser: not tried. W1-06 uses a store-only zip writer that streams to a file where the
  browser offers a save handle, and falls back to a Blob for the rest, with a size warning.

## Conditions for Live mode

1. The relay must carry it: n0's public relays did here, but they are rate limited and not ours. V2-04 (own relays)
   comes before Live links are offered to many people.
2. A Live link needs the serving device's endpoint id and a relay it can be reached on in the link.
3. Re-measure on a phone and in Safari and Firefox before shipping; record the result here.
