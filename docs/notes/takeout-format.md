# Google Takeout: what a Google Photos export holds

Written for I1-01 (Google half) on 2026-11-07 from the public samples, and checked on 2026-11-20 against
a real, current export (see [A current export](#a-current-export-june-2026)). Rules still marked
*(reported)* were in neither.

The reader that implements it is `shoebox_media::takeout` (I1-05); its tests in
`crates/shoebox-media/tests/takeout.rs` have one case per rule below.

## Samples

Downloaded to `~/shoebox-exports/public-samples/` (outside the repository; the AGPL and GPL material is
read, never copied in).

| Sample | What it is | Used for |
|---|---|---|
| immich-go `Takeout1`–`3` | Three small unpacked exports from a Pixel, October 2023: 31 files | sidecar contents, album metadata, copies in year and album folders |
| immich-go `small.lst` | `unzip -l` of a real three-part export from May 2024: 968 entries, no contents | layout, names, cut names, split parts |
| GPTH Neo `test/unit/metadata_matcher_*` | the naming cases another importer handles | supplemental names, `(n)` forms, `.MP` *(reported)* |

Run the reader over them with
`SHOEBOX_TAKEOUT_SAMPLES=~/shoebox-exports/public-samples cargo test -p shoebox-media --test takeout`.
Result on 2026-11-07: all 12 photos of the three small exports and all 483 media files of the May 2024
listing get a sidecar (481 by name, 2 edited copies through their original); nothing ambiguous.

## Layout

```
Takeout/
  archive_browser.html                 ← one per part
  Google Photos/                       ← localised; the space was U+00A0 (no-break) in the 2024 export
    Photos from 2023/                  ← the library by year (localised: "Photos de 2023", …)
      PXL_20231006_063000139.jpg
      PXL_20231006_063000139.jpg.json
    Album test 6-10-23/                ← an album: copies of its photos, each with its own sidecar
      métadonnées.json                 ← the album's metadata; the file name is localised too
      PXL_20231006_063000139.jpg
      PXL_20231006_063000139.jpg.json
```

- **Names cannot be trusted, contents can.** The root, the year folders and the album metadata file
  all have localised names (`Google Photos` with a no-break space, `métadonnées.json`). The reader
  goes by content: a folder with a JSON that has `title` and no `photoTakenTime` is an album; Google
  Photos' folders are the siblings of folders that hold sidecars. A folder without metadata is not an
  album.
- **Album folder names are cut and escaped**: `Album test 6-10-23` holds the album titled
  `Album test 6/10/23`. The title comes from the metadata file; an untitled album (`Sans titre(9)`,
  `"title": ""`) takes its folder's name.
- **Emoji are kept** in folder and file names (`🇵🇹 Lisbonne ❤️ en famille 👨‍👩‍👦‍👦`, 889 entries).
- **Copies**: a photo in an album is in the album's folder *and* (in the 2023 sample) in its year
  folder, same bytes, each with a sidecar. The May 2024 listing has only album folders (that export
  was of albums alone). The reader joins copies with the same name, size and CRC (zip parts) into one
  item; from a folder, without a CRC, the dates in their own sidecars must also agree.
- **Split parts**: the May 2024 export is three zips; **160 of 483** sidecars are in another part than
  their file. Every entry carries its part; the plan is made over all parts at once.
- **Trash and Archive** folders *(reported; in no sample)*: recognised by name in several languages
  and by the sidecar's `trashed` / `archived`. Trashed items are skipped; archived ones import.
  Locked Folder is never exported.

## Sidecar names

Strongest rule first; each rule is tried on every file of the folder before the next, and two files
claiming one sidecar under the same rule are both left unmatched unless the sidecar's `title` names
exactly one of them.

| Rule | Example | Seen in |
|---|---|---|
| Exact, old form | `IMG_8213.JPG` → `IMG_8213.JPG.json` | every sample (481 of 481 sidecars in the 2024 listing) |
| Exact, supplemental | `IMG_2367.jpg` → `IMG_2367.jpg.supplemental-metadata.json` | *(reported)* |
| **Cut at 46 UTF-16 units** before `.json` | `PXL_20230814_201154491.LONG_EXPOSURE-01.COVER.jpg` (49) → `PXL_20230814_201154491.LONG_EXPOSURE-01.COVER..json` (the cut leaves the dot) | 2024 listing |
| | `😀…😋😛.jpg` (25 emoji) → `😀…😋.json`: 23 emoji, 46 UTF-16 units, 29 characters | 2024 listing |
| | `…ACTION_PAN-01.COVER.jpg` (exactly 46) is not cut | 2024 listing |
| | the supplemental suffix is cut the same way: `…_HDR.jpg.supplemental-me.json` | *(reported)* |
| Moved number | `DSC_0238(1).JPG` → `DSC_0238.JPG(1).json` | immich-go Takeout3 |
| | `IMG(1).jpg` → `IMG.jpg.supplemental-metadata(1).json` or `IMG.jpg(1).supplemental-metadata.json` | *(reported)* |
| Pixel `.MP` | `PXL_1.MP` → `PXL_1.MP.jpg.json` | *(reported)* |
| Title | a sidecar whose `title` is the file's name, when no name rule matched | — (fallback) |
| Ignoring case | `Upper.JPG` → `upper.jpg.json` | *(reported)* |
| Edited copy | `IMG_8416-modifié.JPG` has no sidecar: it takes `IMG_8416.JPG`'s | 2024 listing (French), 2 of 2 |
| Live Photo video | `IMG_1.MP4` next to `IMG_1.HEIC` without a sidecar of its own takes the picture's | *(reported)* |

`title` is not always the file's name: one sample's video `PXL_…063909898.LS.mp4` has the title
`TST_…063909898.LS.mp4` (likely edited by the sample's author). The title is therefore only a fallback
and a tie-breaker, never the first rule.

The word for "edited" is localised; the reader knows `edited`, `modifié`, `bearbeitet`, `editado`,
`modificato`, `bewerkt` and a dozen more. An edited copy in a language it does not know imports as a
photo of its own with no export metadata.

## Sidecar fields

```json
{
  "title": "DSC_0238.JPG",
  "description": "",
  "imageViews": "2",
  "creationTime":   { "timestamp": "1697872351", "formatted": "21 oct. 2023, 07:12:31 UTC" },
  "photoTakenTime": { "timestamp": "1696573800", "formatted": "6 oct. 2023, 06:30:00 UTC" },
  "geoData":     { "latitude": 48.8583736, "longitude": 2.291901, "altitude": 82.09, … },
  "geoDataExif": { "latitude": 48.8583736, "longitude": 2.291901, "altitude": 82.09, … },
  "url": "https://photos.google.com/photo/…",
  "googlePhotosOrigin": { "mobileUpload": { "deviceType": "ANDROID_PHONE", … } }
}
```

| Field | Use |
|---|---|
| `photoTakenTime.timestamp` | the date: **UTC seconds as a string** |
| `formatted` | localised text (`21 oct. 2023`): never parsed |
| `creationTime` | when it was uploaded, not taken: ignored |
| `geoData`, else `geoDataExif` | the place; `0, 0` means none (album metadata has `0, 0`) |
| `favorited`, `archived`, `trashed` | booleans *(reported; absent from every sample)* |
| `people[].name` | name tags *(reported)*; suggestions only (I1-11) |
| `description` | kept in the plan; not imported (no record field) |

Album metadata: `title`, `description`, `access`, `date` (album creation), `location`, `geoData` (`0, 0`).

## Dates

The file's own date wins; the export's fills in when the file has none; when both exist and differ by
something other than a time zone (a multiple of 15 minutes, at most 14 hours), the user re-dated it in
Google Photos and the export's wins (`takeout::resolve_taken_at`, owner's decision 2026-10-31).

**Not yet measured** (needs the owner's export and the S25): whether Takeout files are byte-identical to
the phone's originals, and how many sidecar dates differ from the file's, and by how much.

## A current export (June 2026)

The owner's Takeout of 2026-06-01: seven `.tgz` parts (four of ~50 GB from one job, three smaller ones
from others), 373 GB unpacked, 59,588 files, English. Surveyed in place on `nest` with scripts that
print structure only (key names, value types, name patterns with letters and digits masked, counts);
nothing was copied off the machine. `fixtures/exports/google-takeout` (`cargo xtask exports`) has one
anonymised case per finding below, and `checked_in_export_matches_its_truth` runs the reader over it.

**Layout**
- `Takeout/Google Photos/` with a **plain space** this time, beside `Drive/` and
  `YouTube and YouTube Music/`; no `archive_browser.html` in the unpacked folder.
- 14 `Photos from YYYY`, 118 album folders, `Archive`, `Trash`, and Google's own `Failed Videos`
  (videos with sidecars, no metadata file).
- Two JSON files at the root of Google Photos that are neither sidecars nor album metadata:
  `shared_album_comments.json` (`{"sharedAlbumComments": [{text, creationTime, contentUrl}]}`) and
  `user-generated-memory-titles.json` (`{"title": ["…"]}`).
- 8,510 album files are also in a year folder, 1,119 are in no year folder (shared albums mostly).
  Items in `Archive` and `Trash` are in no year folder.
- **Same name, other bytes:** 18 album files share a name with a year-folder file but not its bytes.
  Their twin is the year folder's `NAME(1).EXT`: Takeout numbers names within a folder, so the album and
  the year folder can disagree.

**Album metadata** is `metadata.json` in 116 of 118 albums. Its shapes:

| Shape | Albums |
|---|---|
| `{"title"}` | 54 |
| **`{}`** | 29 |
| `{title, description, access, date}` (+ `enrichments`: `locationEnrichment` or `mapEnrichment`, `latitudeE7`/`longitudeE7`) | 17 |
| `{"title": ""}` | 5 |
| `{"sharedAlbumComments": [{contentOwnerName, creationTime, text \| liked}]}`, with or without `title` | 10 |

So 33 albums have **no `title` key**. The folder name is then the title, as for `""`. Ten titles differ from
their folder name only by escaping: `'`, `:`, `?` → `_`; `/` → `-`.

**Sidecars**: 27,127, all with `title`, `description`, `imageViews`, `creationTime`, `photoTakenTime`,
`geoData`, `url`. Some keys appear only on some sidecars:
- `googlePhotosOrigin` (25,577): `mobileUpload {deviceFolder, deviceType}`, `webUpload {computerUpload}`,
  `fromSharedAlbum {}` or `composition {type: AUTO | MANUAL}`
- `geoDataExif` (11,780: the key is absent otherwise)
- `people` (9,845, `[{name}]`)
- `appSource {androidPackageName}` (2,887)
- `favorited` (387), `archived` (21) and `trashed` (4). These are only ever `true`; the key is absent otherwise.

`geoData` is 0,0 on 15,585.

**Sidecar names**:
- 20,386 are the full `<name>.supplemental-metadata.json`; 51 are the old `<name>.json`.
- Every longer one is **cut at 46 characters** before `.json` (6,447 at exactly 46), which leaves
  `.supplemental-met`, `-metada`, `.supp`, `.s`, … and, for names of 46 or more, no suffix at all.
- **`(n)` goes after the cut suffix**: `DSC_0238(1).JPG` → `DSC_0238.JPG.supplemental-metadata(1).json`
  (216), which makes 49 characters when the suffix was cut. Camera names that already end in `(1)`
  (`20190207_101800(1).jpg`, 56) keep it in the stem, and their title has it too.
- **Media names are cut too**: a name longer than 51 becomes its first 47 characters plus the extension
  (`….x.jpg`, `…_I(1).jpg`). Only the sidecar's `title` keeps the whole name, and the sidecar is the
  title cut at 46.
- `:` in a title is `_` in the file and sidecar names (`IMG_…:xxxx:.jpg` → `IMG_…_xxxx_.jpg`).
- **Videos with no extension**: 16 MP4 files are exported as `20190207_101900` (and `MVIMG_…`); their
  sidecar and title still say `….mp4`.

**Files with no sidecar of their own**:
- 3,476 Pixel **`.MP`** files: the motion photo's video as an MP4 (`ftypisom`), next to a `.MP.jpg` that has the
  sidecar and MotionPhoto XMP.
- 1,236 `.MP4`: Live Photo videos next to a `.JPG` (1,027) or `.HEIC` (195), plus 14 alone.
- 843 `-edited` copies (`-edited.JPG` 732, `.jpg` 88, `.dng` 4, `-EFFECTS-edited.jpg`, `-edited(1).jpg`).
- 235 `(n)` JPEGs.

**Google's creations** have sidecars with `composition`: `-EFFECTS.jpg`, `-COLLAGE.jpg`, `-ANIMATION.gif`,
`…MOVIE.mp4`. Pixel suffixes seen: `.PORTRAIT.jpg`, `.PORTRAIT.ORIGINAL.jpg`, `.NIGHT.jpg`, `.TS.mp4`,
`-CINEMATIC_MOMENT_VIDEO.mp4`. Other media: `.NEF` (1,023), `.HEIC`, `.MOV`, `.MTS`, `.DNG`.

### What the reader had to learn (fixed 2026-11-20)

`checked_in_export_matches_its_truth` first failed on four things the real export has. The reader now
handles them:
1. **Albums whose metadata has no `title` key** (`{}`, comments only: 33 of 118). An album's metadata is
   now any JSON object whose keys are all album keys (`title`, `description`, `access`, `date`,
   `location`, `geoData`, `enrichments`, `sharedAlbumComments`; none at all included), with `title`, if
   present, a string. The folder name is the title when there is none.
2. **The root's two JSON files made "Google Photos" an album.** Such a file only makes an album when its
   folder is a sibling of the folders holding sidecars (of the album folders, in an export with no
   sidecars), and `user-generated-memory-titles.json`'s `title` is an array.
3. **A video with no extension** takes the sidecar whose `title` is its name plus an extension
   (`Rule::NoExtension`, unique match only).
4. **A Pixel `.MP`** beside its `.MP.jpg` is that photo's video (`live_photo_of`, `Rule::LivePhoto`),
   in its albums. `Rule::PixelMp` still matches an `.MP` whose `.MP.jpg` did not come. Live Photo
   videos now join their picture's albums, like edited copies.

## Still to check

- Whether Takeout files are byte-identical to the phone's originals (needs the S25).
- How the seven parts split files from their sidecars (the parts were unpacked into one folder; listing
  the `.tgz` parts takes hours).
- Localised exports: names other than `metadata.json` and `Photos from`, in a current export.
