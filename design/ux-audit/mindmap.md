# Shoebox — UX mind map (as built, Nov 2026)

Everything a person can see or do in the desktop (macOS) and phone (Android) app today.
Tags on each leaf: **platform** `D` desktop · `M` phone · `DM` both, **depth** = taps/clicks from the app's home screen, **use** = how often a typical person needs it (`daily` · `weekly` · `monthly` · `rare` · `once`).

An interactive, collapsible version is in [mindmap.html](mindmap.html).

```mermaid
mindmap
  root((Shoebox))
    First run
      Create a library
        Name this device
        Pick a role: Keeper / Companion / Viewer
        Save the 24-word recovery kit
      Add this device to a library
        Scan QR or paste pairing code
        Check six-digit code
      Restore with recovery phrase
        Peer link + 24 words + role
    Photos
      Timeline by day
        Pinch to change density
        Date rail scrubber
        Pull to refresh
        First-sync progress bar
      Viewer
        Favorite / Info / Similar / Delete
        Swipe, pinch zoom, pull to close
        Live Photo / video playback
        Get original from another device
        Info panel
          Date, camera, size, GPS
          People in this photo
          Copies + Stored on devices
          Keep original on this device
      Selection
        Share / Favorite / Add to album
        Keep on this device / Remove from album
        Delete from all devices
      Favorites
      Recently Deleted
        Restore / Delete now
    Find
      Search
        Recent searches
        Suggestions: people, places, albums, things, cameras
        Chips: dates, kinds, places
        Indexing coverage line
      Similar photos
      People
        Turn on / off
        Name, rename, merge, hide, This is me
        Name a face in the viewer
    Albums and sharing
      Albums
        Create, rename (desktop only)
        Share this album
        Share a link (web link)
          Service setup: address, phrase, invite code
          Kept online vs From my devices
          Full-size, expiry, stop sharing
      Shared albums
        New shared album / Join with invitation
        Photos, Activity, People tabs
        Invite: add photos or only look, QR + code
        Album settings: keep originals, location policy, leave
        Remove member
      Share sheet
        Add to shared album
        Other apps
        Location: remove or keep
    Bring photos in
      Phone camera backup
        Folders to back up
        Only on Wi-Fi / Only while charging
        Keep screen on until done
        Gallery deletions held prompt
      Mac Photos library backup
      Import files or folder (desktop)
      Add from gallery or files (phone)
      Google Photos Takeout import (desktop)
      Watched folders (desktop)
    Safety
      Sync status dot
      Banners: at risk, backup paused, kit not saved, security, update
      Storage
        Copies per photo chart
        Free up space
      Devices
        Activity card + transfer queue
        Device list: connection, what it keeps
        Add a device: role, QR, code check
        Device detail: make admin, change role, remove
      Recovery kit
        Copy this device's link
        Replace recovery phrase
    Settings
      You: your name
      This device: name, role
      What this device keeps: recent window, favourites
      Search indexing + rebuild
      Copies to keep 1-4
      Connection: relays
      Watched folders, Export
      Diagnostics report
      Appearance: theme, text size
      Leave this library
      About: licences, update check
```

## Outline with tags

### 1. First run (`once`)
| Feature | Platform | Depth | Use | Notes |
|---|---|---|---|---|
| Welcome: create / add this device / restore | DM | 0 | once | Three equal-looking doors; "restore" is a text link |
| Name this device + **role picker** (Keeper / Companion / Viewer) | DM | 1 | once | Jargon before the person has any context |
| Recovery kit (24 words, file / copy / print, confirm checkbox) | DM | 2 | once | Mandatory before entering the app |
| Join: scan QR / paste `shoebox-pair1…` code → six-digit check | DM | 1–2 | once | |
| Restore: peer link + 24 words + role | DM | 1 | rare | Needs another device to approve within 2 min |

### 2. Photos (`daily`)
| Feature | Platform | Depth | Use |
|---|---|---|---|
| Timeline grouped by day, pinch density, date rail, pull-to-refresh | DM | 0 | daily |
| Viewer: swipe, zoom, close; ♥ ⓘ ≈ 🗑 in top-right | DM | 1 | daily |
| Viewer info: date, camera, copies, stored-on, people, keep original | DM | 2 | weekly |
| Live Photo / video | DM | 1 | daily |
| Selection: long-press/drag (M), ⌘/⇧-click (D) | DM | 1 | weekly |
| Selection bar: share, ♥, add to album, keep here, delete (icons only) | DM | 2 | weekly |
| Favorites | D sidebar / M avatar menu | 1 / 2 | weekly |
| Recently Deleted (restore, delete now) | D sidebar / M avatar menu | 1 / 2 | rare |

### 3. Find (`weekly`)
| Feature | Platform | Depth | Use |
|---|---|---|---|
| Search field (`/`, ⌘F) with recents + suggestions | D toolbar / M header icon | 0 / 1 | weekly |
| Chips for dates, kinds, places, people, cameras | DM | 1 | weekly |
| Similar photos (≈ in viewer) | DM | 2 | monthly |
| People grid; name, rename, merge, hide, "This is me" | D sidebar / M Albums → People | 1 / 2 | monthly |
| Name a face from the viewer info panel | DM | 3 | monthly |

### 4. Albums & sharing (`weekly` / `monthly`)
| Feature | Platform | Depth | Use |
|---|---|---|---|
| Albums grid + inline "New album name" field | DM | 1 | weekly |
| Album: Share this album / Share a link / Rename (D only) | DM | 2 | monthly |
| Shared albums list: new, join with invitation | DM | 1 | monthly |
| Shared album: Photos / Activity / People tabs, Add photos, Invite, ⚙ | DM | 2 | monthly |
| Invite: role, QR + long code, copy | DM | 3 | monthly |
| Album settings: keep originals, location policy, leave | DM | 3 | rare |
| Web link: service address + phrase + invite code setup, then online/devices, full size, expiry | DM | 3 | rare |
| Share sheet from selection: new shared album / existing / Other apps | DM | 3 | weekly |

### 5. Bring photos in
| Feature | Platform | Depth | Use |
|---|---|---|---|
| Camera backup toggle, folders, Wi-Fi / charging | M (Settings top) | 2 | once, then glance |
| Mac Photos library backup | D (Settings top) | 1 | once |
| Import files / folder, From Google Photos… | D toolbar | 1 | monthly |
| Add photos: from gallery / from files | M header ＋ | 1 | rare |
| Google Takeout import (plan → running → stopped → report) | D | 2 | once |
| Watched folders | D (Settings; also shown on phone with a typed path) | 2 | rare |

### 6. Safety (`weekly` glance, `rare` action)
| Feature | Platform | Depth | Use |
|---|---|---|---|
| Sync status dot / line | D toolbar / M Library header | 0 | daily glance |
| Banners (one at a time): at risk, backup paused, camera backup off, kit not saved, security, update, Takeout stopped | DM (fewer on M) | 0 | as needed |
| Storage: stat cards, copies-per-photo chart, free up space | D sidebar / M avatar menu | 1 / 2 | monthly |
| Devices: activity card, queue, list, add device, detail, role, admin, remove | DM tab/sidebar | 1 | weekly glance |
| Recovery: copy device link, replace phrase | Settings | 2 | rare |

### 7. Settings (one long page, 14 sections, ~4.5 phone screens)
Backup card · You · This device · What this device keeps · Search · Copies to keep · Connection · Watched folders · Export · Help · Appearance · Recovery · Leave this library · About.
Daily/weekly items (backup state) sit next to rare and dangerous ones (relays, replace phrase, leave library) with the same visual weight.

### 8. System surfaces
Toasts (4 s) · global pairing prompt (six-digit check) · "Let this device in?" recovery request (desktop only) · gallery-deletions prompt (phone) · update banner (desktop only) · security alerts (desktop only).
