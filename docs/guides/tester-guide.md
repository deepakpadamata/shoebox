# Shoebox 0.1 · tester guide

Thank you for trying Shoebox. It keeps your photos on your own devices and copies them between those devices, with no cloud
account involved. This preview exists to find out what breaks, so every problem you report helps.

## What to install where

Everything is on the release page for `v0.1.0`. Each file has a SHA-256 checksum in `SHA256SUMS`, and that file is
signed (`SHA256SUMS.minisig`).

| Device | File | Notes |
|---|---|---|
| Mac (Apple silicon or Intel) | `Shoebox_0.1.0_universal.dmg` | Signed and notarised: it opens without warnings. |
| Android phone (arm64, Android 7 or later; HEIC photos need 9) | `Shoebox_0.1.0_android-arm64.apk` | Allow installing from your browser or file manager when Android asks. |
| Windows | `.msi` or `-setup.exe` | **Not signed yet:** SmartScreen warns on first run (*More info → Run anyway*). Not tried on a real machine. |
| Linux desktop | `.AppImage`, `.deb` or `.rpm` | Not tried on a real machine. |
| Always-on box (a home server, a NAS, an old laptop) | the keeper: `curl -fsSL <release url>/install-keeper.sh \| sh` | Linux x86_64 and aarch64, or macOS. See [the keeper guide](keeper-guide.md). |

**Please have at least one keeper**, or a computer that is on most of the time. A photo counts as backed up once a second
device holds it. A phone alone only has its own copy.

## The first ten minutes

1. Install the app on the device with the most photos and choose *Create a library*. Write the recovery phrase on paper:
   it is the only way back in if every device is lost.
2. Add your other devices: *Settings → Backup → Add a device* (on the Mac, *Backup* in the sidebar). On the phone or the
   second computer, choose *Join* and scan the QR code, or type the six-digit code.
3. Turn on backup:
   * on Android, first run offers *Back up camera roll* (camera photos by default; *Settings → Camera* to choose folders);
   * on a Mac, *Settings → Back up the Photos library*, or *Import* for folders.
4. Watch the status chip at the top of *Photos* (the *Backup* row in the Mac sidebar): it says whether every photo has a
   second copy. Tap it for the Backup page.

**Phone navigation.** Four tabs: *Photos · Albums · Search · Settings*. Backup is the first section of Settings.
On the Mac the sidebar lists Photos, Favorites and People, then Albums, then Backup status, Recently Deleted and Settings.

[The user guide](user-guide.md) explains the ideas (keepers, companions, viewers, Recently Deleted, shared albums).

## Known limits in 0.1

* **No iPhone or iPad app yet.** iPhone photos can still come in from a Mac's Photos library or as files.
* **The keeper has been run on x86_64 Linux only.** The aarch64 build (Raspberry Pi) and a keeper on macOS are built but not
  tried.
* **Memory on phones with a large library:** about 2–2.5 KB per photo for the library itself, so about 250 MB at 100,000
  photos. The *first* sync of a large library needs several hundred MB more while it runs. A phone with 4 GB or more is fine;
  older phones with 2 GB may struggle above about 50,000 photos.
* **The Mac's Photos library backup has not yet run against a real library** (only against a test library). Tell us how
  it goes, especially with iCloud Photos turned on.
* **Windows and Linux desktops are untested** on real machines, and the Windows installer is unsigned.
* **Search by meaning (what is in a photo) needs each photo indexed once:**
  * a phone gets the text model in the app, and downloads the image, face and text-reading models once (about 150 MB)
    from the project's release when no other device of yours has them, on Wi-Fi only;
  * a phone with no computer or keeper indexes its own library while it charges: a big library takes a few nights, and
    a long run shows a progress notification. It stops when the phone is hot or unplugged. This is new and has not yet
    run for a night on a real phone: tell us how it goes;
  * a computer or keeper does the same faster, and shares the work with your phones.
* **Importing from Google Photos is new** (Mac and other computers, *＋ Import → From Google Photos…*; not on phones yet).
  It has been tried on public samples and generated test exports, **not on a current real Takeout**: please try one,
  start with a small export, and tell us about photos with a wrong date, a missing album or a file it left out. A
  Takeout needs free disk space for the photos about to be copied in, plus 512 MB. Windows has no free-space check yet.
  The iCloud export is not supported yet.
* **Shared links on the web (Shoebox Cloud)** are only available if the service is running for your group. Ask whoever gave
  you this build.

## When something goes wrong

1. Note what you did, what you expected and what happened instead. A screenshot helps.
2. Make a diagnostics report on the device where it happened: *Settings → Help → Create a diagnostics report*. It holds
   versions, counts and recent errors. It does not hold photos, names, places or file names, and nothing is sent by
   itself. Copy it or save it.
   * On a keeper, run `shoebox-keeper status` and include the output.
3. Send both to the feedback channel you were given.

If the app will not start at all, include the device, its system version and the version of Shoebox (*Settings → About*
on another device, or the name of the file you installed).

## Updating

*Settings → About → Check for new versions* (the keeper's `status`) says when a newer build is out. Nothing installs by
itself: download it from the release page as before. Your library and settings are kept.
