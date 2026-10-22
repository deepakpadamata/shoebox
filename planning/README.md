# Planning · Shoebox MVP

The executable plan: 8 milestones, **82 issues**, in strict execution order for a **solo developer** (decision #20).
The product and technical context is in `docs/`. Read [`05-prototype-plan.md`](../docs/05-prototype-plan.md) first.

```
planning/
├── README.md               ← this roadmap
├── issues/                 ← one file per implementable issue (front-matter + Why/Scope/Acceptance)
├── epics/                  ← v2 epics (V2-01…04) + v1.x backlog, deliberately not broken down
└── scripts/
    └── import_github.py    ← bulk-create labels, milestones and issues with gh (dry run by default)
```

## Timeline

Estimates are focused developer-days. Calendar weeks assume 5 dev-days/week plus a **20% buffer** for
interruptions, review and unknowns.

| Milestone | Issues | Dev-days | Weeks (buffered) | Cumulative weeks |
|---|---|---|---|---|
| M0 · Foundations | 8 | 9.5 | 2.3 | 2 |
| M1 · Sync core | 16 | 48.5 | 11.6 | 14 |
| M2 · Identity, pairing & keeper | 9 | 19.5 | 4.7 | 19 |
| M3 · Desktop app | 17 | 40 | 9.6 | 28 |
| M4 · Mobile apps | 12 | 33.5 | 8.0 | 36 |
| M5 · Shared albums | 8 | 20.5 | 4.9 | 41 |
| M6 · Hardening & beta | 8 | 19 | 4.6 | 46 |
| M7 · MVP release | 4 | 6.5 | 1.6 | 47 |
| **Total** | **82** | **197** | **47** | |

That's roughly **10 months** to MVP for one person, plus the beta's calendar time
(3–4 weeks, overlapping M7) and app-store review.

**Checkpoints**
- **End of M1 (week ~10): backend decision gate (M1-15).** If Loro beats iroh-docs, insert "Switch default backend to Loro" before M2 and re-estimate.
- **End of M2:** first thing you can use yourself (via the dev CLI): two desktops + a Pi keeper backing up a real folder.
- **End of M4:** the full personal backup loop on phones. That's a natural point to dogfood daily.
- **M6-08 beta exit:** no open data-loss bugs and P1s fixed. This is the only gate to release.

## Conventions

- **IDs** are `M<milestone>-<nn>` and match execution order. `depends_on` only ever points at earlier IDs, so
  working through the list top to bottom always works.
- **Estimates** (`estimate_days`) are 0.5–5 days. Anything bigger was split. Re-estimate after M1-15.
- **Labels:** `area:*` (core, store, node, media, keeper, app, ui, mobile, ios, android, security, infra, docs) and
  `type:*` (feature, spike, test, chore, docs).
- **Features:** every MVP feature in `docs/02-features.md` (F-IDs) is covered by at least one issue. Coverage is checked when generating.
- **Definition of done** for every issue: acceptance criteria met, tests in CI, docs updated if behaviour or a decision changed.

## Importing to GitHub

```bash
python3 planning/scripts/import_github.py --repo OWNER/NAME
```

That's a dry run. Add `--apply` to create labels, milestones and issues (and `--include-epics` for the V2 epics).
Re-runs are safe: created issue numbers are stored in `planning/scripts/github-map.json`.

## Issues in execution order

### M0 · Foundations

| ID | Issue | Est. (d) | Depends on |
|---|---|---|---|
| M0-01 | [Pin iroh crate versions and verify APIs](issues/M0-01-pin-iroh-crate-versions-and-verify-apis.md) | 1 | — |
| M0-02 | [Cargo workspace scaffold and licensing](issues/M0-02-cargo-workspace-scaffold-and-licensing.md) | 1 | M0-01 |
| M0-03 | [CI pipeline with license guard](issues/M0-03-ci-pipeline-with-license-guard.md) | 0.5 | M0-02 |
| M0-04 | [Core domain model types](issues/M0-04-core-domain-model-types.md) | 1.5 | M0-02 |
| M0-05 | [Merge rules (LWW, add-wins set, tombstones)](issues/M0-05-merge-rules-lww-add-wins-set-tombstones.md) | 1.5 | M0-04 |
| M0-06 | [Store traits and in-memory backend](issues/M0-06-store-traits-and-in-memory-backend.md) | 1.5 | M0-05 |
| M0-07 | [Store conformance test suite](issues/M0-07-store-conformance-test-suite.md) | 1.5 | M0-06 |
| M0-08 | [Fixture photo corpus](issues/M0-08-fixture-photo-corpus.md) | 1 | M0-02 |

### M1 · Sync core

| ID | Issue | Est. (d) | Depends on |
|---|---|---|---|
| M1-01 | [Node runtime skeleton](issues/M1-01-node-runtime-skeleton.md) | 2 | M0-06 |
| M1-02 | [Testkit: multi-node scenarios](issues/M1-02-testkit-multi-node-scenarios.md) | 2 | M1-01 |
| M1-03 | [Blob store integration](issues/M1-03-blob-store-integration.md) | 2 | M1-01 |
| M1-04 | [Media import pipeline](issues/M1-04-media-import-pipeline.md) | 3 | M0-04, M0-08 |
| M1-05 | [iroh-docs LibraryStore backend](issues/M1-05-iroh-docs-librarystore-backend.md) | 4 | M0-07, M1-01 |
| M1-06 | [Folder import with dedupe](issues/M1-06-folder-import-with-dedupe.md) | 2 | M1-03, M1-04, M1-05 |
| M1-07 | [Developer CLI harness](issues/M1-07-developer-cli-harness.md) | 1 | M1-06 |
| M1-08 | [Library sync between nodes](issues/M1-08-library-sync-between-nodes.md) | 2 | M1-05, M1-02 |
| M1-09 | [LAN discovery, relay config, connection reporting](issues/M1-09-lan-discovery-relay-config-connection-reporting.md) | 1.5 | M1-01 |
| M1-10 | [Gossip presence](issues/M1-10-gossip-presence.md) | 1.5 | M1-08 |
| M1-11 | [Holdings and copy counts](issues/M1-11-holdings-and-copy-counts.md) | 2 | M1-08 |
| M1-12 | [Role-based blob fetcher](issues/M1-12-role-based-blob-fetcher.md) | 3 | M1-11, M1-09 |
| M1-13 | [Delete, Recently Deleted and blob GC](issues/M1-13-delete-recently-deleted-and-blob-gc.md) | 1.5 | M1-11 |
| M1-14 | [Loro backend spike](issues/M1-14-loro-backend-spike.md) | 3 | M0-07, M1-08 |
| M1-15 | [Store benchmark and backend decision](issues/M1-15-store-benchmark-and-backend-decision.md) | 2 | M1-05, M1-14 |

### M2 · Identity, pairing & keeper

| ID | Issue | Est. (d) | Depends on |
|---|---|---|---|
| M2-01 | [Identity: root key, admin certs, signed device list](issues/M2-01-identity-root-key-admin-certs-signed-device-list.md) | 3 | M1-15 |
| M2-02 | [Recovery kit export and restore](issues/M2-02-recovery-kit-export-and-restore.md) | 2 | M2-01 |
| M2-03 | [Enforce device list and revocation](issues/M2-03-enforce-device-list-and-revocation.md) | 3 | M2-01 |
| M2-04 | [Pairing protocol shoebox/pair/0](issues/M2-04-pairing-protocol-shoebox-pair-0.md) | 3 | M2-01, M1-09 |
| M2-05 | [Keeper daemon and control socket](issues/M2-05-keeper-daemon-and-control-socket.md) | 2 | M1-12, M2-04 |
| M2-06 | [Keeper CLI commands](issues/M2-06-keeper-cli-commands.md) | 2 | M2-05 |
| M2-07 | [Keeper service install](issues/M2-07-keeper-service-install.md) | 1 | M2-06 |
| M2-08 | [Safe free-up-space core](issues/M2-08-safe-free-up-space-core.md) | 2 | M1-11 |
| M2-09 | [Keeper release builds](issues/M2-09-keeper-release-builds.md) | 1.5 | M2-07 |

### M3 · Desktop app

| ID | Issue | Est. (d) | Depends on |
|---|---|---|---|
| M3-01 | [Tauri 2 desktop app scaffold](issues/M3-01-tauri-2-desktop-app-scaffold.md) | 2 | M2-06 |
| M3-02 | [Platform keystore for device secrets](issues/M3-02-platform-keystore-for-device-secrets.md) | 1 | M3-01 |
| M3-03 | [Typed command and event bridge](issues/M3-03-typed-command-and-event-bridge.md) | 2 | M3-01 |
| M3-04 | [Blob URI protocol](issues/M3-04-blob-uri-protocol.md) | 1.5 | M3-01 |
| M3-05 | [Design tokens and component library](issues/M3-05-design-tokens-and-component-library.md) | 2 | M3-01 |
| M3-06 | [Library timeline grid](issues/M3-06-library-timeline-grid.md) | 4 | M3-03, M3-04, M3-05 |
| M3-07 | [Desktop onboarding and recovery-kit reminder](issues/M3-07-desktop-onboarding-and-recovery-kit-reminder.md) | 3 | M3-06, M2-02, M2-04 |
| M3-08 | [Photo viewer with on-demand originals](issues/M3-08-photo-viewer-with-on-demand-originals.md) | 3 | M3-06, M1-12 |
| M3-09 | [Info panel, stored-on and pin original](issues/M3-09-info-panel-stored-on-and-pin-original.md) | 1.5 | M3-08 |
| M3-10 | [Video posters and playback](issues/M3-10-video-posters-and-playback.md) | 3 | M3-04, M3-08 |
| M3-11 | [Selection, favorites, delete and Recently Deleted](issues/M3-11-selection-favorites-delete-and-recently-deleted.md) | 2.5 | M3-06, M1-13 |
| M3-12 | [Personal albums](issues/M3-12-personal-albums.md) | 2.5 | M3-11 |
| M3-13 | [Devices view and pairing UI](issues/M3-13-devices-view-and-pairing-ui.md) | 3 | M3-06, M2-04, M2-03 |
| M3-14 | [Storage health and free up space](issues/M3-14-storage-health-and-free-up-space.md) | 2 | M3-06, M2-08 |
| M3-15 | [Settings and library export](issues/M3-15-settings-and-library-export.md) | 2.5 | M3-06 |
| M3-16 | [Sync status, tray icon and banners](issues/M3-16-sync-status-tray-icon-and-banners.md) | 1.5 | M3-13 |
| M3-17 | [Desktop packaging and auto-update](issues/M3-17-desktop-packaging-and-auto-update.md) | 3 | M3-16 |

### M4 · Mobile apps

| ID | Issue | Est. (d) | Depends on |
|---|---|---|---|
| M4-01 | [Mobile targets and responsive layout](issues/M4-01-mobile-targets-and-responsive-layout.md) | 3 | M3-17 |
| M4-02 | [Mobile keystore for device secrets](issues/M4-02-mobile-keystore-for-device-secrets.md) | 1 | M4-01 |
| M4-03 | [photo-library plugin: iOS (PhotoKit)](issues/M4-03-photo-library-plugin-ios-photokit.md) | 5 | M4-01 |
| M4-04 | [photo-library plugin: Android (MediaStore)](issues/M4-04-photo-library-plugin-android-mediastore.md) | 4 | M4-01 |
| M4-05 | [Camera-roll backup pipeline and sync conditions](issues/M4-05-camera-roll-backup-pipeline-and-sync-conditions.md) | 3 | M4-03, M4-04 |
| M4-06 | [Phone role storage policy](issues/M4-06-phone-role-storage-policy.md) | 2 | M4-05, M2-08 |
| M4-07 | [Mobile onboarding, QR scan and join](issues/M4-07-mobile-onboarding-qr-scan-and-join.md) | 3 | M4-01, M3-07 |
| M4-08 | [Background sync: iOS](issues/M4-08-background-sync-ios.md) | 3 | M4-05, M4-02 |
| M4-09 | [Background sync: Android](issues/M4-09-background-sync-android.md) | 2 | M4-05, M4-02 |
| M4-10 | [Finish-sync notifications and states](issues/M4-10-finish-sync-notifications-and-states.md) | 2.5 | M4-08, M4-09 |
| M4-11 | [Mobile gestures and performance](issues/M4-11-mobile-gestures-and-performance.md) | 3 | M4-07 |
| M4-12 | [Mobile packaging and test distribution](issues/M4-12-mobile-packaging-and-test-distribution.md) | 2 | M4-10, M4-11 |

### M5 · Shared albums

| ID | Issue | Est. (d) | Depends on |
|---|---|---|---|
| M5-01 | [AlbumStore backend](issues/M5-01-albumstore-backend.md) | 3 | M1-15, M2-03 |
| M5-02 | [Album invites, join and limits](issues/M5-02-album-invites-join-and-limits.md) | 3 | M5-01, M2-04 |
| M5-03 | [Location prompt and stripped copies](issues/M5-03-location-prompt-and-stripped-copies.md) | 2.5 | M5-01, M3-15 |
| M5-04 | [Shared album fetch policy and keeper keep-originals](issues/M5-04-shared-album-fetch-policy-and-keeper-keep-originals.md) | 2 | M5-02 |
| M5-05 | [Member management and removal](issues/M5-05-member-management-and-removal.md) | 3 | M5-02 |
| M5-06 | [Album activity feed, presence and profiles](issues/M5-06-album-activity-feed-presence-and-profiles.md) | 2 | M5-01, M1-10 |
| M5-07 | [Save shared photo to my library](issues/M5-07-save-shared-photo-to-my-library.md) | 1 | M5-01 |
| M5-08 | [Shared album UI (desktop and mobile)](issues/M5-08-shared-album-ui-desktop-and-mobile.md) | 4 | M5-02, M5-03, M5-05, M5-06, M5-07 |

### M6 · Hardening & beta

| ID | Issue | Est. (d) | Depends on |
|---|---|---|---|
| M6-01 | [Real-network nightly test](issues/M6-01-real-network-nightly-test.md) | 2 | M5-08 |
| M6-02 | [Scale and endurance testing](issues/M6-02-scale-and-endurance-testing.md) | 3 | M6-01 |
| M6-03 | [Failure scenario suite](issues/M6-03-failure-scenario-suite.md) | 3 | M6-01 |
| M6-04 | [Security review and threat model](issues/M6-04-security-review-and-threat-model.md) | 3 | M5-08 |
| M6-05 | [On-disk format versioning and migrations](issues/M6-05-on-disk-format-versioning-and-migrations.md) | 2 | M5-08 |
| M6-06 | [Empty states, first-sync progress and accessibility](issues/M6-06-empty-states-first-sync-progress-and-accessibility.md) | 2.5 | M5-08 |
| M6-07 | [Diagnostics bundle](issues/M6-07-diagnostics-bundle.md) | 1.5 | M6-04 |
| M6-08 | [Closed beta](issues/M6-08-closed-beta.md) | 2 | M6-02, M6-03, M6-05, M6-06, M6-07 |

### M7 · MVP release

| ID | Issue | Est. (d) | Depends on |
|---|---|---|---|
| M7-01 | [User and self-hosting documentation](issues/M7-01-user-and-self-hosting-documentation.md) | 2 | M6-08 |
| M7-02 | [Website and download page](issues/M7-02-website-and-download-page.md) | 1.5 | M7-01 |
| M7-03 | [App Store and Play submission](issues/M7-03-app-store-and-play-submission.md) | 2 | M6-08 |
| M7-04 | [MVP 1.0 release](issues/M7-04-mvp-1-0-release.md) | 1 | M7-02, M7-03 |

## After the MVP

- [v1.x backlog](epics/V1X-backlog.md): Live Photos/RAW pairs, hidden album, Takeout import, smart albums, one-off send, comments, integrity check.
- v2 epics: [self-hosted web viewer](epics/V2-01-self-hosted-web-viewer.md), [hosted gateway](epics/V2-02-hosted-gateway-relay-view.md),
  [zero-knowledge backup](epics/V2-03-hosted-zero-knowledge-backup.md), [own relays](epics/V2-04-own-relay-infrastructure.md).
