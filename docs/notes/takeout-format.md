# Google Takeout: what a Google Photos export holds

Written for I1-01 (Google half) on 2026-11-07 from the public samples. **No current export has been
checked yet**: every sample predates the `.supplemental-metadata.json` names, so the rules for those
come from other tools' reports and are marked *(reported)*. Check this page against a current export
from the owner before I1 closes.

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

## Still to check against a current export

- `.supplemental-metadata.json` names and their cuts.
- Trash, Archive and their names; `favorited`, `archived`, `trashed`; `people`.
- An edited copy with a moved number (`IMG-edited(1).jpg`).
- Whether long media file names are cut too.
- A Live Photo and a motion photo uploaded from an iPhone and a Pixel.
