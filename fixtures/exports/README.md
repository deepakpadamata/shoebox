# Export fixtures

Two minimal, anonymised exports for import work (I1), laid out like real ones:

| Folder | What | Modelled on |
|---|---|---|
| `google-takeout/` | A Google Takeout, unpacked: `Takeout/Google Photos/…` plus a little of two other products | a real 7-part, 373 GB Takeout from June 2026 (27,127 sidecars, 118 albums) |
| `icloud/` | An iCloud Photos data export, unpacked: `iCloud Photos/Photos/…`, `Memories/…`, `Albums/…` | a real export from November 2026 (367 files) and Apple's *Data and Privacy File Guides* |

Regenerate both with `cargo xtask exports` (`xtask/src/exports.rs`); same code, same bytes. Each folder has
`truth.json`, one entry per media file: what an import must make of it, why (`case`) and whether that case was
**seen** in the real export (`"seen": true`) or only documented by Google, Apple or other tools (`false`).
`crates/shoebox-media/tests/takeout.rs` (`checked_in_export_matches_its_truth`) runs the Takeout reader over
`google-takeout/` against its truth; `crates/shoebox-media/tests/icloud.rs` does the same for the iCloud reader over
`icloud/`, and `crates/shoebox-node/tests/import_archive.rs` imports `icloud/` into a library.

## What is real and what is not

Nothing here was copied from a real export. File names follow the real patterns (`IMG_<stamp>.jpg`,
`PXL_<stamp>.MP.jpg`, `DSC_0101(1).JPG`, …) with invented dates, album titles are invented ("Beach Day"),
people are "Person A", places are city centres. Pixels come from `xtask` and the checked-in samples, so the whole set
is **CC0-1.0** like `fixtures/samples`. What *is* taken from the real exports is the structure: folder and file
naming, sidecar name cuts, JSON keys, their order and their value types, CSV columns and date formats.

Two shortcuts stand in for things the generator cannot make small:
- `.NEF` files hold the repository's synthetic DNG stand-in (a TIFF, not a decodable RAW).
- The Pixel `.MP.jpg` is a plain JPEG. A real one carries MotionPhoto XMP and its video at the end of the file.

## Google Takeout: the cases

Year folders: plain photo with `people` and a place; the old `<name>.json` sidecar; the same name twice in a
year in both real `(1)` forms; `-edited` copies (both cases of extension); a video exported with **no
extension**; a title with `:` that the file names spell `_`; a 65-character name (file cut to 47 + extension,
sidecar = title cut at 46) and a 51-character one (sidecar cut); sidecar suffixes cut at 46
(`.supplemental-metada`, `-met`, `-m`, `.supplemental`); a Pixel motion photo (`.MP.jpg` with a sidecar,
`.MP` without one); Live Photos (`.HEIC` + `.MP4`, `.JPG` + `.MP4`); Google creations (`-EFFECTS`,
`-ANIMATION.gif`); a favourite with no place; a screenshot whose date and place come from the export; two
different `DSC_0200.NEF`; a photo from a shared album.

`Trash/` (trashed), `Archive/` (archived), `Failed Videos/` (no album metadata): items in these are in no
year folder.

Albums, one per real metadata shape: full metadata with a location enrichment (`Beach Day`); title only, with
an escaped folder name (`Mom_s 60th` → "Mom's 60th"); `/` → `-` with a map enrichment (`Trip 6-10-23`);
**`{}`** (`Weekend Hike`); `"title": ""` (`Untitled(3)`); shared album with comments and a photo in no year
folder (`Shared Picnic`); comments and **no title** (`Road Trip`). `Beach Day/DSC_0200.NEF` has the bytes of the
year's `DSC_0200(1).NEF`, not of the year's `DSC_0200.NEF`. Takeout numbers the year folder only.

Not media: `shared_album_comments.json` and `user-generated-memory-titles.json` at the root of Google Photos,
`Takeout/Drive/` (a JPEG that is not part of Google Photos), `Takeout/YouTube and YouTube Music/`.

## iCloud: the cases

`Photos/Photo Details.csv` with the real columns
(`imgName,fileChecksum,favorite,hidden,deleted,originalCreationDate,viewCount,importDate`), LF line endings, no
BOM, rows in no order, dates as `"Monday January 26,2015 12:53 PM GMT"`. Media files:
- seen in the real export: an iPad photo whose EXIF is local time with no zone; a JPEG with no EXIF; a PNG
  screenshot; a lone MOV;
- documented but not seen: `favorite`/`hidden`/`deleted` = `yes`, a Live Photo (`.HEIC` + `.MOV`), a CSV row whose
  file is missing.

`Memories/26 January 2015.csv` (header `imageName`, as seen) is ignored by the import. `Albums/Trip 2015.csv`
(header `Images`) follows Apple's guides; the real export had no albums.

See `docs/notes/takeout-format.md` and `docs/notes/icloud-format.md` for what the real exports showed.
