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
| Everyday (phone) | previews and thumbnails of everything, originals of what you pin or took on that device | phones and laptops |
| Viewer | previews only, never originals | a family tablet |

**Copy count is the number to watch.** Every photo shows how many devices hold its original. The goal is
two or more (you can change it in Settings), **or one copy on a keeper**: a keeper is the backstop, so a photo
it holds counts as backed up. A photo only on a phone is one lost phone away from gone; the app tells you
how many photos are in that state and when they will be safe.

**Photos are only ever deleted by you.** *Delete* moves a photo to Recently Deleted for 30 days on every
device. Freeing space on a phone removes only originals that at least two other devices confirmed holding.

## First run

1. **Create a library** on your first device. Give it a name and a role.
2. **Save the recovery kit.** You are shown 24 words (and a printable page). They are the only way back if
   *every* device is lost. Store them somewhere safe and offline, then confirm. A banner reminds you until you do.
3. **Add a second device.** On the first device open *Devices → Add a device*, choose its role and show the
   code. On the new device choose *Add this computer to an existing library*, scan or paste the code, then
   check that both screens show **the same six digits** and tap *They match*. If the digits differ, tap *They
   do not match*: someone is in the middle.
4. **Bring photos in.** On a computer: *Import* a folder (the app can also watch a folder). On a phone: turn
   on camera backup in Settings; it uploads new photos to your other devices when conditions allow
   (Wi-Fi only, charging only and battery limits are yours to choose).

## Sharing an album

*Shared* lists the albums you share with other people (who may be on their own libraries).

* **Create** a shared album, then **Invite**: choose whether the person can add photos or only look. You get
  a QR code and a text code; the person opens *Shared → Join with an invitation* and scans or pastes it. Both
  of you need to be online while they join. An invitation works once and for seven days.
* **Adding photos.** If a photo has a location, you are asked to *keep* or *remove* it before everyone sees where
  it was taken, and you can remember the choice for the album. Removing the location changes only the shared
  copy; your own photo keeps it. Some formats (such as HEIC) cannot be cleaned yet and are left out.
* **Save to my library** copies a shared photo into yours.
* **Remove someone** (owners only, in the People tab): the album moves to a new private space and everyone
  else follows automatically. The person removed keeps the photos they already downloaded: that cannot be
  undone.
* **Leave** an album from its settings. You keep anything you saved to your library.

## When something goes wrong

* **A device was lost or stolen.** On another device: *Devices → Remove* next to it. The remaining devices
  move to a new library space the lost device cannot read. If the lost device was an **admin** (the first
  phone and the first computer are), or you remove it from a device that is not one, you are asked for your
  24-word recovery phrase: that way a thief holding an admin device cannot remove you instead.
* **All devices were lost.** Install Shoebox, choose *I lost my devices*, enter the 24 words and the link of
  any device that still has the library. (A keeper prints its link with `shoebox-keeper address`.)
* **Photos show "not backed up".** You need another device online. Add a keeper, or open the app on your
  laptop while it is on the same Wi-Fi.
* **Ask for help.** *Settings → Help → Create a diagnostics report* makes a text with versions, counts and
  recent errors. It contains no photos, names, places or file names. Nothing is sent: copy it or save it.

## Your data is yours

*Settings → Export* copies every original on that device into plain folders by year and month with a
`library.json` for favorites and albums. If you stop using Shoebox you lose nothing.

## Privacy in one paragraph

Devices connect directly to each other, end-to-end encrypted. When two devices cannot reach each other
directly they go through a relay that only forwards encrypted bytes; the relay can see that two devices talk and
how much, not what. You can run your own relay (`relay-guide.md`). Shoebox collects no analytics and has no
accounts.
