# Shoebox user guide

Shoebox keeps your photos on *your* devices and keeps them in sync directly between those devices. There is no
account and no company server holding your library.

## The ideas worth knowing

**Your devices are your account.** A "library" is the set of devices you have joined together. Whichever
device you start the library on is the first member; you add others by scanning a code. If you lose a
device, you remove it from another one.

**Roles decide what each device keeps.**

| Role | Keeps | Good for |
|---|---|---|
| Keeper | every original, always | a computer or small always-on box (Raspberry Pi, NAS, a cheap server) |
| Companion | previews and thumbnails of everything, plus the full-size photos you pin, favourite (optional) or that were taken in the last 90 days (adjustable); older ones are fetched from a keeper when you open them | phones, tablets and laptops |
| Viewer | previews only, never originals | a family tablet |

A role is not forever: *Devices → Role* next to a device changes it, for example when an old laptop becomes the
keeper. It takes an admin device (the one you created the library on, and any device restored with the
recovery phrase), or your recovery phrase from any other device. *Devices → Make admin* (with the phrase) gives
you a second admin, so losing one device does not leave every change needing the phrase. The device switches as soon as it hears about the change.

**Copy count is the number to watch.** Every photo shows how many devices hold its original. The goal is
two or more (you can change it in Settings), **or one copy on a keeper**: a keeper is the backstop, so a photo
it holds counts as backed up. A photo only on a phone is one lost phone away from gone; the app tells you
how many photos are in that state and when they will be safe.

**Photos are only ever deleted by you.** *Delete* moves a photo to Recently Deleted for 30 days on every
device. Freeing space on a Companion removes only originals that a keeper, or at least two other devices, confirmed holding
in the last week: the same rule as "backed up". Photos from the phone's own gallery are not freed this way: Shoebox
keeps no copy of them, only the gallery does. Free-up also leaves alone what the Companion keeps by its own rules: recent photos, favourites (if kept) and pins.

**What a Companion keeps** is yours to set in *Settings → What this device keeps*: the recent window (off, 30 days, 90 days, 1 year), and whether favourites are kept. Pins are always kept. Changing the window changes what is fetched; nothing is removed until you choose *Free up space*.

## First run

1. **Create a library** on your first device. Give it a name; the device's role is chosen for you (a phone is a companion, a computer a keeper) and can be changed with *Change*.
2. **Save the recovery kit.** You are shown 24 words (and a printable page). They are the only way back if
   *every* device is lost. Store them somewhere safe and offline, then confirm. A banner reminds you until you do.
3. **Add a second device.** On the first device open *Settings → Backup → Add a device* (the QR code comes first; *Send pairing link instead* shares the text code). On the new device choose *Add this computer to an existing library*, scan or paste the code, then
   check that both screens show **the same six digits** and tap *They match*. If the digits differ, tap *They
   do not match*: someone is in the middle.
4. **Bring photos in.** First run ends with a *Bring your photos in* step (camera backup, adding a device, Google Photos). On a computer: *Import* a folder (the app can also watch a folder). On a phone: turn
   on camera backup in Settings; it uploads new photos to your other devices when conditions allow
   (Wi-Fi only, charging only and battery limits are yours to choose). Shoebox reads gallery photos where they are
   and keeps no second copy on the phone; delete a photo in the gallery and it is deleted from your library too
   (Recently Deleted, on every device). To add photos the backup does not see (albums you have not shared, a cloud
   drive), tap **＋** in the Library.

   **Moving over from Google Photos (on a Mac or other computer).** Ask Google for your photos at
   takeout.google.com: choose *Deselect all*, tick only *Google Photos*, pick *.zip* and the largest file size, and
   wait for Google's email (a few hours to a few days). Download every part into one folder. In Shoebox choose
   **＋ Import → From Google Photos…**, select all the parts at once (or the folder, if you unpacked them), and read
   the summary before you start: how many photos, which albums, and the space needed against the space free. The
   import keeps what Google knew: dates you corrected, places, favourites and albums; photos in Google Photos' bin
   are left out, and a photo that appears in several albums is imported once. It runs in the background and you can
   close the window; *Stop* pauses it, and if you quit the app the next start offers *Continue*. When it ends you
   get a report of what came over and what did not. The files are copied into your library and read only once; your
   downloaded parts stay where they are, so you can delete them afterwards.

## Sharing an album

*Albums* lists your albums and the ones shared with you in one list (shared ones carry a *Shared* label and a badge for new photos).

* **Your name.** People you share with see the name in *Settings → You*, the same on all your devices (until you set
  one, they see the device's name).
* **All your devices take part.** Join or create a shared album on one device and your other devices join it by
  themselves; the other people see you once, not once per device. Leaving on one device leaves on all of them.
* **Share an album you already have:** open it under *Albums* and choose *Share*, then *Invite people*. Its photos are shared,
  and photos you add to the album later are shared too. (Removing a photo from your album does not remove it from
  the shared one.)
* **Create** a shared album, then **Invite**: choose whether the person can add photos or only look. You get
  a QR code and a text code; the person opens *Albums → ＋ → Join with an invitation* and scans or pastes it. *Send invitation…* hands the text to the system share sheet. Both
  of you need to be online while they join. An invitation works once and for seven days.
* **Adding photos.** If a photo has a location, you are asked to *keep* or *remove* it before everyone sees where
  it was taken, and you can remember the choice for the album. Removing the location changes only the shared
  copy; your own photo keeps it. Some formats (such as HEIC) cannot be cleaned yet and are left out.
* **Save to my library** copies a shared photo into yours.
* **Remove someone** (owners only, in the People tab): the album moves to a new private space and everyone
  else follows automatically. The person removed keeps the photos they already downloaded: that cannot be
  undone.
* **Leave** an album from its settings. You keep anything you saved to your library.

**New versions.** The app tells you when a newer Shoebox is out (a banner with a download link); install it over the
old one and your library stays. It asks the Shoebox website twice a day and sends nothing about you; turn it off in
*Settings → About*. A keeper mentions new versions in `shoebox-keeper status`.

## When something goes wrong

* **A device was lost or stolen.** On another device: *Backup → Devices*, open it and choose *Remove*. The remaining devices
  move to a new library space the lost device cannot read. If the lost device was an **admin** (the one
  you created the library on is), or you remove it from a device that is not one, you are asked for your
  24-word recovery phrase: that way a thief holding an admin device cannot remove you instead.
* **All devices were lost.** Install Shoebox, choose *I lost my devices*, enter the 24 words and the link of
  any device that still has the library. (A keeper prints its link with `shoebox-keeper address`.) If that
  device has a screen, it asks whether to let the new device in: allow it there within two minutes.
* **Someone may have seen your recovery phrase.** *Settings → Advanced → Replace recovery phrase*, enter the current one,
  and save the new kit it shows (once). Your devices switch to the new phrase as soon as they are in touch, and
  the old one opens nothing. Remove any device you do not recognise first.
* **"Your library's recovery phrase was replaced".** Shown on your other devices after a replacement. If it
  was you, get the new phrase to wherever you keep it; if not, check Backup → Devices for devices you do not know.
* **"… became an admin of your library".** Shown on every device when a new admin appears, for example when
  someone restores with the recovery phrase. If it was not you, someone has your phrase: remove that device
  (Backup → Devices → Remove, with the phrase) and keep the phrase somewhere safer.
* **The status chip says "Only on this phone" or "At risk".** You need another device online. Add a keeper, or open the app on your
  laptop while it is on the same Wi-Fi.
* **Ask for help.** *Settings → Help → Create a diagnostics report* makes a text with versions, counts and
  recent errors. It contains no photos, names, places or file names. Nothing is sent: copy it or save it.

## Searching

Type in the search field (`/` or ⌘F on a computer, the magnifier on a phone). Shoebox understands dates and ranges
("2019", "March 2021", "last summer", "weekends", "Christmas"), "videos", "screenshots", "selfies", "favourites",
cameras, albums and places, and shows each as a chip you can remove. Other words are matched against what is in the
photos ("dog on a beach"), labels, text in the photos and names. Searches run on the device you are holding: they work
with every other device switched off.

Finding photos by what is in them needs each photo to be indexed once, by any of your devices: a keeper does it all
the time, a computer when it is plugged in and you are away, a phone while it charges. A phone with nothing else does it by itself, a few nights for a large library, and *Settings → Search* says how long is left. Until then the line under the
results says how far it has come. *Settings → Search* turns indexing on this device on or off and rebuilds this
device's index if something looks wrong. In the viewer, ≈ shows similar photos.

## People

Faces are found and grouped on your own devices and never leave them. *People* (in the sidebar on a computer, in
Albums on a phone) shows the groups: give a group a name and its photos are found by that name ("Ana", "Ana and
me" once you said which person is you). In a photo's info panel you can name a face or say it is not someone. People
can be turned off for the whole library at the top of the People screen.

## Your data is yours

*Settings → Export* copies every original on that device into plain folders by year and month with a
`library.json` for favorites and albums. If you stop using Shoebox you lose nothing.

## Privacy in one paragraph

Devices connect directly to each other, end-to-end encrypted. When two devices cannot reach each other
directly they go through a relay that only forwards encrypted bytes; the relay can see that two devices talk and
how much, not what. You can run your own relay (`relay-guide.md`). Shoebox collects no analytics and has no
accounts.
