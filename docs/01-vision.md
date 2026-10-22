# 01 · Vision

## One-liner

A photo library that behaves like iCloud/Google Photos — automatic backup, every photo on
every device, easy sharing — but where the "cloud" is just the devices you and your people
already own, connected directly with iroh.

## Who it's for

| Persona | Need |
|---|---|
| **The household archivist** | Has a phone, a laptop and an old desktop/NAS. Wants every family photo backed up in ≥2 places without paying for cloud storage or trusting a provider. |
| **The privacy-minded photographer** | Doesn't want their library scanned, mined or locked into a vendor. Wants originals, not recompressed copies. |
| **The family group** | Wants a shared album for a trip/kid/event where everyone can add photos, without everyone signing up for the same service. |

## Principles

1. **Devices are the account.** Identity is a keypair generated on device. Adding a device
   = pairing with a QR code. There is no sign-up and no password.
2. **Local-first.** The app is fully usable offline. Sync is opportunistic and resumable.
3. **Originals are sacred.** Content-addressed (BLAKE3) storage, bit-exact originals, verified
   on transfer. Never recompress the original.
4. **Make redundancy visible.** The user should always be able to answer "if I lose this
   phone, what do I lose?" Copy-count is a first-class UI concept.
5. **Share by capability, not by account.** Sharing = handing someone a ticket (link/QR) that
   grants view or contribute rights to a specific album.
6. **Connectivity is honest.** Peer-to-peer means sometimes the other device is asleep. The UI
   shows who's online, what's pending, and never pretends something is backed up when it isn't.

## Non-goals (for v1)

- Not a social network (no public feeds, likes, followers).
- Not a photo editor (basic rotate/crop at most, non-destructive, later).
- No hosted service is *required*. In v2 we may offer optional hosted services: a gateway "relay view"
  for sharing with people without the app, and a zero-knowledge cloud backup that only ever holds ciphertext.
  Everything must keep working without them.
- No server-side ML. Any search/faces happen on device, later phase.
