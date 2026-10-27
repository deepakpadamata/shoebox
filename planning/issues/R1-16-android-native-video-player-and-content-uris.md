---
id: R1-16
title: Android native video player and content:// for local media
milestone: R1 · v0.1 testers' preview
labels: [area:android, area:mobile, area:ui, type:feature]
estimate_days: 4
depends_on: [R1-09]
features: [F2.9]
---

# R1-16 · Android native video player and content:// for local media

## Why

Android's WebView re-applies byte ranges to custom-scheme responses, so a long video cannot play from `shoebox://`
(A-098). v0.1 works around it with a token-gated loopback HTTP server inside the app (`src-tauri/src/media.rs`). That
works, but it is a listening port, a cleartext exception in the network security config and a CSP entry, and it routes
every byte of a video the phone already holds through Rust and a socket. The owner chose the native route (2026-10-27):
play videos in Android's own player, and show anything already on the phone straight from where it lives.

## Scope

- **Local media from where it lives.** A photo or video whose original is in the phone's gallery (an in-place
  reference, A-092) is shown from its MediaStore `content://` URI: images in the viewer, videos in the player. Nothing
  is copied or streamed through the app core for these. Check first whether the WebView loads `content://` images
  from the app's page (CSP `img-src content:`, `allowContentAccess`); if it does not, images stay on `shoebox://`
  (small, unranged, unaffected by A-098) and only video goes native.
- **Native player.** A small Tauri plugin (Kotlin, AndroidX Media3/ExoPlayer) plays a video over the viewer: open,
  close, play/pause, seek, the system back gesture, rotation, and audio focus. The viewer asks the plugin instead of
  rendering `<video>` on Android; desktop keeps `<video>` over `shoebox://`, which works there.
- **Originals that are not on the phone** are fetched first (the existing `fetch_now`, with progress) and played from
  the local copy, not streamed from another device while playing. Where the fetched copy lives in the blob store,
  hand the player that file if the store gives a stable path, or a Media3 `DataSource` that reads ranges from the
  core over JNI (one `export_ranges` per read, A-098).
- **Poster frames** from the platform (`MediaMetadataRetriever` on the `content://` URI or file) instead of decoding a
  frame in the WebView (`ui/src/lib/poster.ts`).
- **Remove the workaround** once the above works: `media.rs`, the `media_base` command and `videoUrl`, the
  `127.0.0.1` cleartext exception, and `http://127.0.0.1:*` in the CSP. Record the outcome in A-098.

## Acceptance criteria

- [ ] On a physical phone, a 140 MB 1080p HEVC video taken on that phone starts within 1 s from its `content://` URI,
      seeks without stalling and plays to the end; nothing is read through the app core for it.
- [ ] A video held only on a keeper is fetched with progress and then plays from the phone's copy.
- [ ] A gallery photo's viewer image comes from its `content://` URI (or the check above shows why it cannot).
- [ ] Every video in the grid has a poster frame made natively.
- [ ] The app no longer opens a listening port; the release network security config allows no cleartext.

---
_Depends on: R1-09 · Estimate: 4d · Features: F2.9_
