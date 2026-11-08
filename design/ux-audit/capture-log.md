# Capture log

## Android — `screenshots/android/` (Samsung SM-S931B, Android, live library: 5,275 photos, 3 devices, dark theme unless noted)

> These are from the owner's real library and contain personal photos, faces, names and folder names. Keep the folder out of any public copy of the repo.

| File | Screen | Reached by | Notes |
|---|---|---|---|
| 01-library-top / 01-library | Library timeline | app launch | "Syncing…" permanently while keeper offline; 1-photo days take a full row |
| 02-device-menu | Avatar menu "This device" | avatar | Favorites/Deleted/Storage/Settings hidden here |
| 03-add-photos-sheet | Add photos | header ＋ | gallery / files |
| 04-viewer | Viewer (Live Photo) | tap tile | ♥ ⓘ ≈ 🗑 top-right, no Share |
| 05-viewer-info | Viewer info panel | ⓘ | copies pill, stored on, keep original |
| 06-selection | Selection mode | long-press | icon-only bar |
| 07-share-sheet | Share 3 photos | selection → share | new shared album field first, "Other apps…" second |
| 08-add-to-album-sheet | Add to album | selection → ⊞ | |
| 10-settings-1…4 | Settings (whole page) | avatar → Settings | 4½ screens; desktop paths on phone (2, 3) |
| 11-backup-folders-sheet | Folders to back up | Settings → Choose folders | Back key leaves Settings instead of closing |
| 20-search-empty | Search, recents | header ⌕ | |
| 21-search-suggest | Suggestions | typing "beach" | |
| 22-search-results | Results "the words beach" | Enter | keyboard stays up |
| 23-search-chips | Chips "videos" + "last summer" | Enter | |
| 30-albums | Albums | tab | People row + create field on top; cover placeholder |
| 31-album-detail | Album "The boys" | tap album | title reads "Album" |
| 32-people / 32b | People named + unnamed | Albums → People | 2 per row, links under each |
| 40-shared | Shared (empty) | tab | |
| 41-new-shared-album-sheet | New shared album | ＋ New shared album | |
| 42-join-invite-sheet | Join a shared album | Join with an invitation | |
| 50-devices | Devices | tab | activity card + 3 devices |
| 51-devices-queue | Devices with queue | Show queue | |
| 52-device-detail-sheet | S25 detail | tap device | "Make admin…" on an admin |
| 54-add-device-sheet | Add a device: role | ＋ Add a device | |
| 60-favorites | Favorites | avatar → Favorites | |
| 61-recently-deleted | Recently Deleted (empty) | avatar | |
| 62-storage | Storage | avatar → Storage | "at least 1 copies" |
| 90-light-library / 91-light-devices | Light theme | system light mode | status-bar icons invisible |

Not captured on the phone (kept to reversible actions): pairing QR (tap did not register; desktop has it), change role, make admin, remove device, replace phrase, leave library, deletion confirm, gallery-deletions prompt, onboarding (desktop has it).

## Desktop — `screenshots/desktop/` (same UI as the Mac app, served by `desktop-dev-server` against a throwaway demo library of synthetic images; light theme unless noted)

The live Mac window could not be driven (no computer-use tools in this session) and captured blank from another Space, so the desktop tour ran in the in-app browser at 1280×800. The native app differs only where a native picker or the Mac Photos library is involved (Import sheet shows file/folder buttons instead of a typed path; Settings starts with the Photos library backup card).

| File | Screen | Notes |
|---|---|---|
| 00-onboarding-welcome | Welcome | three doors |
| 01-onboarding-name-role | Name + role | Keeper/Companion/Viewer up front |
| 02-onboarding-recovery-kit | Recovery kit | demo phrase, throwaway library |
| 03-library-empty | Empty library | Import / Add a device |
| 04-import-sheet-browser-variant | Import (browser variant) | native app shows pickers |
| 05-google-import-idle | Import from Google Photos | Takeout steps |
| 06-library-timeline | Library | |
| 07-viewer-info | Viewer + info column | "1 copy · wants 2" |
| 08-selection-bar | Selection | icon-only bar |
| 09-albums | Albums | cover placeholder, People row duplicates sidebar |
| 10-album-detail | Album | two share buttons + Rename; no Add photos |
| 11-share-link-setup | Share a link | asks for service URL first |
| 12-shared-album | Shared album | jumped from Albums to Shared section |
| 13-invite-sheet / 14-invite-code | Invite → QR + long code | |
| 15-album-settings | Album settings | |
| 16-shared-list | Shared albums | broken cover image |
| 17-people-empty | People (no faces yet) | |
| 18-devices | Devices (only device) | "Everything is backed up" |
| 19-storage | Storage | banner contradicts chart |
| 20-settings-1…4 | Settings | 520 px column, half window empty |
| 21-search-no-results | Search with chip | |
| 22-device-menu | Device ⋯ menu | |
| 23-add-device-role / 24-add-device-qr | Add a device | |
| 25-library-dark-stale-search | Dark theme | old query left in search field |
