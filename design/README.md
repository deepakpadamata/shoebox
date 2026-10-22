# Design

Open `mockups/index.html` in a browser (no build step; plain HTML + `assets/mockup.css` / `assets/mockup.js`).
Photos are generated gradients; QR codes are fake.

## Screen inventory

| File | Screens | Features |
|---|---|---|
| `01-onboarding.html` | 1a Welcome · 1b Name & recovery kit · 1c Join existing library (scan + code check) · 1d Import & device role | F1.1–F1.5, F2.1, F4.1, F4.3, F8.1 |
| `02-library.html` | 2a Timeline (mobile) · 2b Multi-select actions · 2d Recovery kit reminder · 2c Library (desktop) | F1.4, F2.2–F2.8, F3.5, F4.2, F4.4, F5.2, F7.1 |
| `03-viewer.html` | 3a Viewer streaming original · 3b Info & where-stored · 3c Original unavailable | F3.1–F3.5 |
| `04-albums-sharing.html` | 4a Albums · 4b Shared album feed · 4c Invite · 4d Accept invite · 4e Location prompt | F5.1–F5.3, F6.1–F6.5, F6.9, F8.3 |
| `05-devices.html` | 5a Devices & activity · 5b Add device · 5c Device detail · **Background sync:** 5d Lock-screen nudge · 5e Finishing in foreground · 5f Nothing reachable · **Headless keeper:** 5g CLI pairing · 5h Confirm on phone · 5i `status` | F4.1–F4.7, F6.9, F7.1, F8.1, F8.4 |
| `06-storage-settings.html` | 6a Library health & free up space · 6b Settings · 6c Relay servers | F7.1–F7.4, F8.1–F8.5 |

## Navigation (mobile)

Tabs: **Library · Albums · Shared · Devices**. Settings from the avatar in the Library header.

- *Shared* is intended as a cross-album activity feed ("Ana added 12 to Family"). Not mocked yet —
  candidate to fold into Albums if it feels thin.

## Design language

- **Calm by default, loud when at risk.** Photos without badges are safe. Red "1 copy" = only on one device;
  blue ↻ = in transit.
- **Connection honesty.** Green dot = direct (incl. LAN), amber = via relay, grey = offline.
- **Roles over settings.** Keeper / Phone / Viewer instead of a pile of storage toggles.
- Accent colour is cardboard amber (the shoebox). Everything else is neutral so photos carry the colour.

## Rules for the new states

- **Banner priority on Library:** backup paused (5f, explains why photos are at risk) > at-risk "1 copy" (2a) > recovery kit reminder (2d). Show only one at a time.
- **"Open the app" notification:** only when at-risk photos exist, a peer is reachable, and background time
  ran out. At most once a day. Never sent when nothing is reachable (5f).
- **Keeper pairing runs in reverse:** the phone shows the ticket, the keeper dials it, and the phone confirms and signs.

## Not yet mocked

- Recently Deleted, search, video player, smart albums
- Shared tab feed, member management (remove member / leave album)
- Desktop versions of devices, sharing and settings
- Empty states and first-sync progress (big initial transfer)
- v2 hosted services: cloud backup setup, gateway web view
