# 04 · Decisions & open questions

Decisions from the first review pass (2026-10-22). Each one notes where it shows up in the other docs and mockups.

## Decisions

### Product

| # | Question | Decision | Affects |
|---|---|---|---|
| 1 | Name | Keep **Shoebox** as the working name; revisit before any public release. | — |
| 2 | Recovery kit in onboarding | **Skippable, with a persistent banner** until it's saved. | F1.4, mockup 1b |
| 3 | Default role for a new phone | **Phone**: previews of everything, plus originals of its own captures until they meet the redundancy target. | F4.3, mockup 1d |
| 4 | Headless keeper in MVP? | **Yes.** An always-on keeper (NAS / Pi / VPS daemon) is MVP. It's what makes P2P sync reliable in practice. | F4.6 → MVP |
| 5 | Shared albums & keeper storage | **Opt-in per album.** Shared albums keep previews by default; a per-album "Keep originals" toggle makes my keeper hold every member's originals. | new F6.9, mockup 4d |
| 6 | Web viewer / hosted services | **v2.** A self-hosted web viewer served by a keeper, plus an optional **hosted gateway "relay view"** and an optional **hosted private cloud backup** service. The two can be used together. The backup is **zero-knowledge** (ciphertext only; it can't see photos or generate previews), so the web view can't be powered by the backup service alone. | F6.8, new F7.7, vision non-goals |

### Technical

| # | Question | Decision | Affects |
|---|---|---|---|
| 7 | iroh-docs vs own CRDT | Metadata sync sits **behind a `LibraryStore` interface**. **Benchmarked in M1-15 (see `docs/notes/store-benchmark.md`):** Loro syncs 100k photos in 0.8 s (500k in 6.8 s) and persists 100k in 60 MB; iroh-docs as used (one entry + one blob per record) takes 29 s at 60k and does not complete at 100k. **Decision: personal library → Loro (issue M1-16); shared albums → iroh-docs** (needs its capability model, albums are bounded). To be reviewed before M1-16 starts. | architecture §Building blocks, M1-15 |
| 8 | Location stripping vs content addressing | **Share originals by default.** When adding photos that contain location to a shared album, **prompt** the user, who can strip location. Stripped items become derived copies with their own hash; the album records `originalHash → sharedHash`. | F8.3, mockups 4c/4e/6b |
| 9 | Library identity | **Per-device keys + a signed device list.** A library root key (kept in the recovery kit) signs device additions and revocations. | F1.4, F4.5, architecture §Identity |
| 10 | Mobile background sync | **Best-effort.** Opportunistic OS background tasks, a clear "open the app to finish syncing" state, and the keeper as the redundancy backstop. No real-time promise. | F4.4, architecture §Platforms |
| 11 | UI stack | **Tauri 2 everywhere** (desktop + iOS + Android) with the Rust core in-process. | architecture §Platforms |
| 12 | Relays | **n0 public relays for prototype/MVP.** Run our own once hosted services ship in v2. Custom relay URL always available. | F8.2 |

### Design

| # | Question | Decision | Affects |
|---|---|---|---|
| 13 | Copy-count badges | **Only at-risk ("1 copy") and syncing items** get a badge. | F2.6, mockup 2a/2c |
| 14 | First prototype target | **Desktop and mobile at once**, sharing the Rust core and Tauri UI (see #11). Includes the headless keeper (#4). | roadmap |

### Planning (second pass, 2026-10-22)

| # | Question | Decision | Affects |
|---|---|---|---|
| 15 | Frontend framework | **Svelte 5 + Vite** inside Tauri 2. | 05 §6, M3 |
| 16 | Minimum OS | **iOS 16 / Android 10**; desktop macOS 12+, Windows 10+, Ubuntu 22.04+ (and equivalents). | M3, M4 |
| 17 | License | **Apache-2.0 OR MIT** for library crates (`shoebox-core`, `-node`, `-store-*`, `-media`, `-testkit`); **AGPL-3.0** for the apps (`keeper`, `desktop-mobile`). v2 hosted services can stay ours. A license check in CI keeps AGPL code out of the library crates. | M0 |
| 18 | Root key custody | **Delegated admin certs.** The root key signs admin certs (by default for the first phone and the first desktop). Admin devices can pair and revoke. Otherwise the root key lives only in the recovery kit. | 05 §4, M2 |
| 19 | Shared album limits (MVP) | **50 members, 50k items** per album. These are soft limits, enforced in the UI and the invite flow. | M5 |
| 20 | Delivery assumptions | **Solo developer, strictly linear.** Issues are numbered in execution order. The MVP is planned in detail; v2 (hosted gateway, cloud backup, web viewer) is kept as epics. | `planning/` |
| 21 | v0.1 scope (2026-10-26) | **A testers' preview of macOS, Android and the keeper on x86_64 Linux.** iOS is out of v0.1 (no Xcode, no iOS tester). The keeper is tested on the owner's Linux box, not a Raspberry Pi. The code goes to GitHub only once the rest of 0.1 is done, so CI, signed builds and the update feed come last. | `planning/` R1 |

## Still open

- **Hosted services business model** (#6). Free tier? How is the backup service paid for? How does the gateway authenticate album viewers?
- **Web view source of truth** (#6). The backup is zero-knowledge, so the gateway "relay view" has to fetch from an online member device or keeper, or be given album-specific read keys. Decide which.
- **Background sync on iOS** (#10). Validate what Tauri 2 can reach of BGTaskScheduler / BGProcessingTask, or whether a small native plugin is needed.
- **iroh-docs status** (#7). In 2025 n0 said it wasn't ready for 1.0. Check the current repo/changelog before the spike.
- **Shared album limits** (part of old #5). Max members/items for v1 are not decided yet.
- **Metadata backend** (#7). Decided by M1-15: Loro for the personal library, iroh-docs for shared albums (review pending, see the benchmark note).
- **Exact iroh crate versions.** Pinned in M0-01. Neither iroh-docs' status nor which versions work together could be confirmed from public sources during planning.
