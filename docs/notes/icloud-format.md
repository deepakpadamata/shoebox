# iCloud Photos data export: what it holds

Written for I1-01 (iCloud half) on 2026-11-20 from the owner's export (privacy.apple.com → *Request a copy of your
data*, November 2026) and Apple's *Data and Privacy File Guides* that came with it. The reader that will implement it
is I1-06 (`shoebox_media::icloud`, A-147). `fixtures/exports/icloud` (`cargo xtask exports`) is the
anonymised fixture, one case per finding below. Its `truth.json` marks each case **seen** or only documented.

The export was read where it lies (two zips in the repository root, untracked: never commit them). Only names,
columns, counts and date offsets were looked at.

## What came

Two zips:

| Zip | Contents |
|---|---|
| `iCloud Photos.zip` (416 MB) | `iCloud Photos/Photos/`: 365 `IMG_nnnn.JPG`, 1 `.PNG`, 1 `.MOV`, and `Photo Details.csv`; `iCloud Photos/Memories/`: two CSVs named by date (`11 November 2015.csv`) |
| `icloud-photos.zip` (8 MB) | Six PDF guides: *Photo Details*, *Favourites*, *Hidden*, *Memories*, *Shared Albums: Album Info*, *Subscribed Albums* |

The library is an iPad's camera roll from 2014–15: **no HEIC, no Live Photos, no albums, no favourites, no hidden or
deleted photos**. So no `Albums/` folder came, and neither did `Favorites.csv`, `Hidden.csv` or any shared album
file. Their formats below come from the guides only. The zip had a single root with no "Part 1 of N" in its name. How
a larger library is split into parts is not known yet.

## `Photo Details.csv`

```
imgName,fileChecksum,favorite,hidden,deleted,originalCreationDate,viewCount,importDate
IMG_0388.MOV,AVCW…(28 chars),no,no,no,"Thursday September 24,2015 11:37 AM GMT",0,"Thursday September 24,2015 11:37 AM GMT"
```

- UTF-8, **no BOM**, LF line endings, one row per file, **in no particular order**; every file in `Photos/` has a
  row and every row a file.
- `favorite`, `hidden`, `deleted`: `yes` / `no` (only `no` seen).
- `fileChecksum`: 28 base64 characters, all starting with `A`. It is Apple's own hash, not a hash of the file we can
  recompute. Carry it, never check against it.
- `viewCount`: `0` everywhere.
- **Dates**: `"<Weekday> <Month> <d>,<yyyy> <h>:<mm> <AM|PM> GMT"`. English, quoted because of the comma, **no space
  after the comma**, **minutes only**. `importDate` equals `originalCreationDate` on every row.
- **The date is UTC cut to the minute.** Against the photos' EXIF `DateTimeOriginal` (local time, no zone, no
  `OffsetTimeOriginal`): all 315 photos with one are 330 or 331 minutes later than the CSV (+05:30 plus the cut
  seconds). 50 JPEGs have no EXIF at all, and neither does the PNG; for those the CSV is the only date. The I1 rule
  (the file's date wins unless the export differs by something other than a zone) therefore has to allow the CSV's
  missing seconds. A strict "multiple of 15 minutes" test would wrongly take the CSV's date for every photo.

Apple's guide (January 2020) lists only `imgName` and `Import Date`. The real file has more columns than it
documents.

## Media

- Names are the device's `IMG_nnnn.EXT`, upper-case extensions. No folders inside `Photos/`.
- JPEG EXIF: `Make` Apple (314) or another camera (1). No GPS anywhere in this library.
- The MOV carries QuickTime `creation_time` (UTC) and `com.apple.quicktime.creationdate` (with its zone).

## Memories

`Memories/<d Month yyyy>.csv`, header `imageName`, one file name per line. The import ignores them (I1 decision).

## Documented, not seen

From the guides (2019–2020) and the I1 epic. Check them when an export with these comes:

| File | Format (per Apple) |
|---|---|
| `Favorites.csv` | header `Images`, one name per line |
| `Hidden.csv` | header `Images` |
| Albums | `Albums/<title>.csv`, one name per line (header `Images`, per the guides; other tools report `imgName` or `imageName`) |
| `AlbumInfo.json` (shared albums you own) | `albumName`, `creationDate`, `allowContributions`, `isPublic`, `owner {fullName, appleid}`, `photos [{name, dateCreated, contributor}]`, `comments [...]`, `participants [...]` |
| `Subscribed Albums.json` | `albumName`, `files [{filename, dateCreated}]`, `comments [...]` |
| Live Photos | `IMG_nnnn.HEIC` + `IMG_nnnn.MOV`, same stem |

Shared albums, Memories and edits are not carried over by the import (I1 decision). Their files are only listed here
so the reader can recognise and skip them.
