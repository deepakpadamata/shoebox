# Recipients adding photos to a web link (W1-11, spike)

Written 2026-11-03. **Recommendation: defer.** Build it after Cloud links have real users and the free service has
survived a month of open registration (A-118). The manifest already reserves what it needs; nothing else changes now.

## What was tried

`crates/shoebox-core/tests/inbox_spike.rs`: a sealed box, throwaway and not part of any format. The page needs only an
*inbox public key* (X25519, 32 bytes) from the manifest. It makes a fresh X25519 key, multiplies the inbox key by it,
hashes the shared secret with both public keys into a key, seals the file with XChaCha20-Poly1305 (the cipher blobs
already use) and sends the ephemeral public key in front. A member device holding the inbox secret opens it. Checked: a
3 MB file seals with 48 bytes of overhead and opens on the device; the public key, another inbox and a flipped bit do
not open it; two uploads of one file differ.

**Not done, which the issue asked for:** the throwaway *page* and the upload route. Only the cryptography is shown, as a
Rust test; in a browser it is an X25519 multiplication (WebCrypto has X25519 in current browsers, or the WebAssembly
the page already loads) and the cipher the page already has. No browser ran it.

## The four questions

**Where does the ciphertext go?** To the service, as a blob under the share: `PUT /s/{share}/inbox/{n}`, which is
the one public *write*, so it needs the most care: the share must opt in (the manifest carries the inbox key only then),
a per-share byte and file count cap, a size cap per file, and nothing is served back to the public. Streaming to a
Live device is possible but fails while the device is off, which is the rule S1 set aside; Live links get no uploads.
Uploads sit in a separate area, not in the owner's quota until accepted, and are deleted after 30 days.

**How does it get into the album?** The inbox secret is a library setting (so every member device has it, like the
share key). A member's app lists "4 photos from a guest" with a thumbnail made after opening them on the device, and
each photo is accepted or rejected. Accepting imports it through the normal path (so it gets a thumbnail, a date and
the usual checks) and adds it to the album; the link refreshes, as for any album change. Nothing enters unaccepted, and
nothing is opened by the service.

**Abuse.** The service cannot see what is uploaded, so limits are what it has: the owner's toggle (off by default, per
link), a cap of 50 files and 200 MB a link, 25 MB a file, a type check *by the accepting device*, expiry with the link,
and the existing rate limits plus a tighter one on writes. Unwanted material we cannot see reaches only the owner, who
rejects it; the report button (A-118) switches the whole link off. A service that accepts anonymous uploads of
unreadable data from anyone with a link is a larger risk than one that only serves; that is the main reason to wait.

**Identity and quota.** A typed name only, shown as "from {name}" and unverified. Accepted photos become the accepting
member's photos and use their quota (the cloud copy, if they have it on). Pending uploads use the link's own cap, not
the owner's quota.

## What the manifest reserves now

`inbox_key: Option<[u8; 32]>` already exists in `Manifest` (W1-03) and is `None` today. It is the X25519 *public* key
and nothing else; the secret stays on the devices. The sealed-box layout above (`ephemeral public key ‖ ciphertext`)
is not versioned and would get its own domain string and a line in `formats.md` when built. No other field is needed.

## When to build it

Build when someone asks for it twice, with a week of design on the write route first (limits, deletion, a test that
the service holds no readable upload, the accept screen). Drop it if the free service has abuse problems that reports
alone do not handle.
