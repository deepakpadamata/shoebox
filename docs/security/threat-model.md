# Threat model and self-review (M6-04)

Status: a **self-review** of the code as of this commit. No external review has been done; one is
recommended before the public release (see "Not covered").

## What we protect

| Asset | Where it lives | Why it matters |
|---|---|---|
| Photo and video bytes | each device's blob store | the whole point |
| Library metadata (dates, places in EXIF summaries, albums, favorites) | Loro document on each device | reveals habits and places |
| Device key (iroh endpoint secret) | OS keystore (macOS Keychain, Windows Credential Manager, Secret Service, Android Keystore-wrapped), file fallback `0600` | it is the device's identity; anyone with it can pose as the device |
| Library root key | **nowhere**: shown once as the 24-word recovery phrase, re-derived only to sign | can add admins and devices |
| Admin/device certificates | in the library, signed by the root | decide who may sync |
| Shared-album capabilities | `shared-albums.json` (`0600`), inside invitations | the key to an album |
| Invitations and pairing tickets | on screen / pasted | bearer secrets until used |

## Who we defend against

| Attacker | Can do | Mitigations | Residual risk |
|---|---|---|---|
| **A lost or stolen phone/laptop** | read what is on it; try to join the network as that device | OS keystore holds the device key; an admin removes it (revocation + epoch rotation: the others move to a new library space the stolen device cannot read, and refuse its connections); photos on the device itself are protected only by the OS's own disk encryption and lock | until revoked, the thief can sync with the others; photos already on the device are exposed |
| **A malicious or careless album member** | share an album's content onward; as a *contributor* write anything into the album (A-078) | viewers hold a read-only capability; the owner can remove a member (the album moves to a new space); location can be stripped from what is shared | a contributor is trusted not to tamper; what a member downloaded cannot be taken back |
| **A network attacker** | observe or tamper with traffic, try man-in-the-middle at pairing | iroh/QUIC with the peer's key as identity (TLS 1.3); pairing adds a 6-digit code derived (keyed BLAKE3) from the ticket secret and both nonces, so a man in the middle without the ticket cannot make the digits match; tickets are single-use, expire in 5 minutes and failed attempts are rate-limited | a 6-digit code gives about 1-in-a-million odds per attempt *within* the rate limit; if the ticket itself is shown to an attacker they can race the intended device |
| **The relay operator** | see that two keys talk, when, and how much | traffic is end-to-end encrypted; relays only forward; self-hosting a relay is supported (`docs/guides/relay.md`) | metadata (who talks to whom) is visible to the relay and to anyone on the path |
| **Someone who gets an invitation** | join the album it names | one use, 7 days, per-invitation random secret, inviter checks it before letting the joiner fetch anything | a faster interceptor wins (it is a bearer token) |
| **A removed device or member** | keep what it downloaded; try to reconnect | connection gate (`PeerTracker`) refuses devices not in the device list; album peers are dropped from the blob gate when the album moves; rotation makes the old space useless | cannot be un-downloaded |
| **A malicious peer sending bad data** | corrupt blobs, malformed records, oversize frames | every blob is verified (BLAKE3) as it streams; records are signed and verified in parallel; frames are length-capped; the codec rejects unknown versions | resource exhaustion by an authorised member (an owner could write a huge album) |
| **Whoever receives a diagnostics report** | read it | built to contain counts and roles only, with names, paths, coordinates, file names and invitations removed (tested) | a report still says roughly how big a library is |

## Review checklist and findings

| Area | What was checked | Result |
|---|---|---|
| Ticket entropy and expiry | pairing ticket: 256-bit secret from the OS RNG, expires (default 5 min), single use, per-peer attempt limiter. Album invitation: 128-bit random secret, 7 days by default, single use, expiry checked on both sides | OK |
| SAS derivation | `blake3::keyed_hash(ticket secret, "shoebox/pair-sas/0" ‖ ids ‖ both nonces)` mod 10⁶; both sides compute and compare | OK (6 digits is the usual trade-off; the ticket secret never crosses the wire) |
| Certificate chain | device certificates are signed by an admin certificate, admin certificates by the root; revoked devices are listed; unsigned and out-of-chain records are rejected on every replica (`judge`, parallel verification); rotation notices are verified against the root and the identity id | OK |
| Capability handling | the root key is never stored; library capability holds no secret (Loro backend); the album write capability is a secret: **F1 (fixed)** the registry holding it was created with default permissions, now `0600` and tested | Fixed |
| Keystore usage | iOS/macOS Keychain, Windows, Secret Service via `keyring`; Android Keystore AES-GCM wrapping with migration of plaintext keys; `allowBackup=false` so keys never reach cloud backup | OK (desktop keystore paths compiled but never run: not verified) |
| Log redaction | tickets/phrases are never passed to `tracing`; the diagnostics bundle redacts names, paths, coordinates, file names, e-mail addresses, invitations (`redact` tests include a hostile line) | OK |
| Connection gate | with a library, only device-list peers connect; outsiders get pairing, recovery, the album handshake, docs/gossip (guarded by the namespace capability) and — only for album members — blob transfer. **F2 (fixed while building M5)** an earlier draft let album members use any protocol; the allow-list is now per protocol | Fixed |
| Dev HTTP bridge | `desktop-dev-server` exposes every command on `127.0.0.1` with no authentication | **F3 (accepted)**: it exists only behind the `http` cargo feature, which no release build enables; documented in the README |
| WebView surface | CSP limits scripts, images and media to the app, `shoebox:` and `blob:`; the `shoebox://` handler serves only photos that are in the library or in an album the device belongs to | OK |
| Android surface | the debug-only `BurstNow` receiver is not in release builds; the camera permission is requested only when scanning | OK |
| Album tampering | contributors hold the write capability (A-078) | **F4 (accepted)**: members you invite to contribute are trusted; the owner's remedy is to remove them. Not fixable without per-record author signatures in albums (a post-MVP item) |
| Invitation guest window | a stranger with a valid invitation can fetch blobs from the inviter for 30 minutes even if it never finishes joining | **F5 (accepted)**: it can only fetch blobs whose hashes it knows, which for a stranger is none until it reads the album |
| Blob hash knowledge | an album member can fetch *any* blob from you if it knows the hash | **F6 (accepted)**: hashes are 256-bit and appear only in records the member can read |
| Location removal | GPS removal covers JPEG, PNG and MP4/MOV; not HEIC, embedded thumbnails or maker notes (A-080) | **F7 (accepted, user-visible)**: unsupported files are refused, not shared with their location |

Severity: nothing found that lets a stranger read photos or impersonate a device. F1 and F2 were
the two issues with real consequences and are fixed. F4 is the one a user should understand
(a contributor can misbehave inside their shared album).

## Not covered

* No cryptographic review of the dependencies (iroh, loro, ed25519 implementations) beyond using
  their maintained releases and `cargo deny` for licences and sources.
* No fuzzing of the wire formats beyond the property tests in `shoebox-core` and the codec tests.
* Denial of service by a member of the device list (they are trusted with the library).
* A compromised OS or a malicious app on the same device.
* Traffic analysis.

Recommended before 1.0: an external review of pairing/recovery/rotation and the album handshake, and
a fuzzing pass over `entry_codec`, the Loro wire frames and the strip-location parsers.
