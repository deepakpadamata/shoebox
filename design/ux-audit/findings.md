# Shoebox — UX audit findings

Method: walked every reachable screen on the Android app (Samsung S25, live library: 5,275 photos, 3 devices) and the desktop UI (same Svelte code as the Mac app, run through `desktop-dev-server` against a throwaway demo library, because computer-use was not available to drive the Mac window). Cross-checked against the code in `apps/desktop-mobile/ui/src`. Screenshots are in `screenshots/android/` and `screenshots/desktop/` (index in [capture-log.md](capture-log.md)).

Severity: **S1** blocks or misleads on a core job · **S2** real friction on a common path · **S3** polish / consistency.

## The core jobs, ranked by frequency

| # | Job | How often | Taps today (phone) | Verdict |
|---|---|---|---|---|
| 1 | Look at my photos | daily | 0 | Good — fast grid, pinch, date rail |
| 2 | Know they are safe | glance daily, act rarely | 0 to read "Syncing…", 1–2 to get a real answer | **Unclear** — four places, contradicting each other |
| 3 | Send a photo to someone | weekly | 3+ (no share in viewer; select → share → "Other apps…") | **Too deep** |
| 4 | Find a photo | weekly | 1 | Good engine; entry is a small icon |
| 5 | Put photos in an album / share an album | monthly | 3–4 | Two parallel album worlds |
| 6 | Add a device | rare | 2 | OK, but role jargon up front |
| 7 | Tweak settings | rare | 2 + long scroll | Flat 14-section page |

The app is strong on 1 and 4, weak on 2 and 3 — and 2 is the whole promise of the product.

---

## S1 — misleading or blocking

**F1. Three different answers to "are my photos safe?"** (`desktop/19-storage`, `desktop/18-devices`, `desktop/07-viewer-info`)
On a one-device library with copies-to-keep = 2: Storage's banner says *"Every photo has at least 2 copies"* while its own chart directly below shows **all 201 photos at 1 copy**; Devices says *"Everything is backed up"*; the viewer shows a red *"1 copy · wants 2"*; tiles show no risk badge; the toolbar says *"Up to date"*. The rule "a copy on a keeper counts as safe" is applied in some places and not others, and never explained where the number is shown. Principle 4 of the vision ("make redundancy visible") is undermined.

**F2. Sharing a single photo is buried.** (`android/04-viewer`, `android/07-share-sheet`)
The viewer has ♥ ⓘ ≈ 🗑 but **no Share**. From the grid you must long-press → select → share icon → a sheet whose first, largest control is a "New shared album…" text field; the thing people want 90 % of the time (send to WhatsApp / Messages / AirDrop) is the secondary "Other apps…" button.

**F3. Destructive and everyday actions share the same weight and reach.** (`android/04-viewer`, `android/06-selection`)
Delete sits one tap away in the viewer's top-right cluster next to Info and Similar, same size and style (only the icon is red). In the selection bar "Keep on this device" is an unlabeled download arrow next to the delete can. Icons-only bars rely on recognition the icons don't earn (≈, ⤓, ⊞).

**F4. Phone shows desktop-only controls that cannot work.** (`android/10-settings-2`, `-3`)
Settings → Watched folders and Export on Android show text fields pre-filled with `/Users/you/Pictures/Inbox` and `/Users/you/Shoebox export`. A phone user can't type a meaningful path; these should be hidden or replaced by a system picker.

## S2 — friction on common paths

**F5. Albums vs Shared are two separate worlds.** (`desktop/09-albums`, `desktop/10-album-detail`, `desktop/12-shared-album`, `android/30-albums`, `android/40-shared`)
An album has two share buttons ("Share this album", "Share a link") that do completely different things; pressing the first silently moves you to a different section (Albums → Shared) with a different layout (tabs, tiny ⚙, role pill). Shared albums have their own simpler viewer (`SharedAlbum.svelte`) with different controls than the main viewer. Users must learn two models for one idea.

**F6. Album covers never show.** (`android/30-albums`, `desktop/09-albums`, `desktop/16-shared-list`)
Album cards always show the ▤ placeholder (shared albums show a broken-image icon) even with photos inside. In `core.rs` the cover is a stored LWW field initialised to `None` and nothing in the UI sets it; there is no fallback to the first/newest photo. Album grids are unscannable.

**F7. Mobile album page is titled "Album", not the album's name.** (`android/31-album-detail`) Desktop shows the name; mobile shows the generic word. No rename on mobile either.

**F8. Navigation hides weekly things and surfaces rare ones.** (`android/02-device-menu`)
Phone tabs are Library · Albums · Shared · Devices. Favorites (weekly) lives behind the avatar in a sheet titled **"This device"** (it isn't device-specific), next to Recently Deleted, Storage and Settings. People is only reachable via a row at the top of Albums. Meanwhile "Devices" — something most people look at rarely and act on even more rarely — gets a whole tab. Search, a weekly job, is a 36 px icon.

**F9. Settings is one flat 4½-screen page.** (`android/10-settings-1…4`, `desktop/20-settings-1…4`)
14 sections with equal visual weight: camera backup (important) next to relays, rebuild index, replace recovery phrase, leave library (rare, risky). No grouping, no search, no drill-in. On desktop the column is 520 px wide with half the window empty.

**F10. First-run asks for jargon before value.** (`desktop/01-onboarding-name-role`)
"What does this device do? Keeper / Companion / Viewer" appears before the person has seen a single photo. The defaults are already right (Keeper on computers, Companion on phones); the choice could be deferred to Devices. The same picker appears again in "Add a device".

**F11. Web link setup starts with "Service address https://".** (`desktop/11-share-link-setup`)
The first thing "Share a link" asks is a URL, then the recovery phrase, then an invite code. For most people this is a dead end. If no service is configured, the option should say so and explain, or offer "From my devices" without setup.

**F12. Invitations and pairing codes are long opaque strings.** (`desktop/14-invite-code`, `desktop/24-add-device-qr`)
`SHOEBOX-ALBUM1-…` and `shoebox-pair1…` codes are 300+ characters in a textarea. QR is right for in-person, but there is no "Send invitation…" via the system share sheet for remote friends.

**F13. Android Back from a sheet leaves the screen.** (observed on Settings → Folders to back up)
System Back while a bottom sheet is open navigated back to Library instead of just closing the sheet. Sheets should consume Back (and support predictive back).

**F14. Search keeps the keyboard up after Enter, and the desktop query sticks around.** (`android/23-search-chips`, `desktop/25-library-dark-stale-search`)
On phone the results are half-covered by the keyboard after submitting. On desktop the last query ("gif 2026") stays in the toolbar field after navigating to Devices or Library.

**F15. Two different "Syncing / Up to date" signals with no explanation.**
Phone Library shows "● Syncing…" permanently while the keeper is offline; nothing says what is syncing or that something is waiting. Devices explains it ("Waiting for nest · 15 originals not on it yet") but is a different tab.

## S3 — consistency and polish

- **F16.** Theme choice isn't re-applied on launch: `localStorage.theme` is only read in `Settings.svelte`; `main.ts` never applies it.
- **F17.** Android status-bar icons are white on the light theme (invisible) — `android/90-light-library`.
- **F18.** "Storage" (sidebar, title) vs "health" (route `#/health`) and "check Health first" (Leave dialog text, `Settings.svelte:333`).
- **F19.** Copy: "at least 1 copies" (Storage, `android/62-storage`); missing space "space.Photos" (Health free-up text).
- **F20.** Mixed icon sets: emoji 🗑 👥 render in colour next to monochrome glyphs ▦ ▤ ⇄ ◔ ⚙︎ — inconsistent weight and alignment (`android/02-device-menu`, tab bar).
- **F21.** People grid on phone is 2 large circles per row with "Rename · Merge…" links under every face — low density and noisy (`android/32-people`).
- **F22.** "Make admin…" offered on the device that already is an admin (`android/52-device-detail-sheet`); "Keep original on this device" offered on a keeper that keeps everything (`desktop/07-viewer-info`).
- **F23.** Sparse days waste space: a day with one photo takes a full row plus a header (`android/01-library-top`). Month grouping when zoomed out would help.
- **F24.** Platform parity gaps: desktop-only security, update and recovery-request surfaces; phone recovery-kit banner can't be dismissed; no album rename on phone.
- **F25.** Undefined `--danger` token in `WebLinkSheet.svelte`; DateRail, SearchField, ShareSheet, WebLinkSheet, BackupCard missing from the `#/dev` gallery.
- **F26.** Small text: much secondary copy is 12–12.5 px with `--ink-3` on `--surface`; fine on desktop, tight on a phone held at arm's length.

## What works well (keep)

- Calm visual language, neutral chrome, amber accent; photos carry the colour.
- Timeline performance, pinch density, date rail, long-press drag-select.
- Search chips that show how a query was understood, plus suggestions by type.
- Devices list: connection honesty (Same network / Via relay / Offline) and per-device storage bars.
- Honest, plain-language copy throughout ("Nothing goes through a company's cloud").
- One-banner-at-a-time priority rule.
