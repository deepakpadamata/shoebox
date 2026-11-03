//! W1-11 (spike, throwaway): can a browser with nothing but the link seal one file so that only a member device can
//! open it? A sealed box: an ephemeral X25519 key, the shared secret hashed with both public keys into an
//! XChaCha20-Poly1305 key, the ephemeral public key in front. Nothing here is part of any format; the note is
//! `docs/notes/web-viewer-uploads.md`. In a browser the same steps are `crypto.getRandomValues`, an X25519 scalar
//! multiplication (WebCrypto has it in current browsers, or the WebAssembly build the page already loads) and the
//! cipher the page already has for blobs.

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use curve25519_dalek::constants::X25519_BASEPOINT;
use curve25519_dalek::montgomery::MontgomeryPoint;

fn public_of(secret: &[u8; 32]) -> [u8; 32] {
    X25519_BASEPOINT.mul_clamped(*secret).to_bytes()
}

fn key(shared: &[u8; 32], eph_pub: &[u8; 32], inbox_pub: &[u8; 32]) -> [u8; 32] {
    let mut h = blake3::Hasher::new_derive_key("shoebox/spike/inbox-box/v0");
    h.update(shared);
    h.update(eph_pub);
    h.update(inbox_pub);
    *h.finalize().as_bytes()
}

/// What the page does: needs only the inbox public key from the manifest.
fn seal(inbox_pub: &[u8; 32], plain: &[u8]) -> Vec<u8> {
    let mut eph = [0u8; 32];
    getrandom::fill(&mut eph).unwrap();
    let eph_pub = public_of(&eph);
    let shared = MontgomeryPoint(*inbox_pub).mul_clamped(eph).to_bytes();
    let k = key(&shared, &eph_pub, inbox_pub);
    // A one-use key, so a fixed nonce is safe; the cipher is the one blobs use.
    let ct = XChaCha20Poly1305::new((&k).into())
        .encrypt(&XNonce::from([0u8; 24]), plain)
        .unwrap();
    [eph_pub.to_vec(), ct].concat()
}

/// What a member device does with the inbox secret.
fn open(inbox_secret: &[u8; 32], sealed: &[u8]) -> Option<Vec<u8>> {
    let (eph_pub, ct) = sealed.split_at_checked(32)?;
    let eph_pub: [u8; 32] = eph_pub.try_into().ok()?;
    let inbox_pub = public_of(inbox_secret);
    let shared = MontgomeryPoint(eph_pub)
        .mul_clamped(*inbox_secret)
        .to_bytes();
    let k = key(&shared, &eph_pub, &inbox_pub);
    XChaCha20Poly1305::new((&k).into())
        .decrypt(&XNonce::from([0u8; 24]), ct)
        .ok()
}

#[test]
fn a_file_sealed_with_only_the_inbox_public_key_opens_on_a_member_device_and_nowhere_else() {
    let mut inbox_secret = [0u8; 32];
    getrandom::fill(&mut inbox_secret).unwrap();
    let inbox_pub = public_of(&inbox_secret);

    let photo = vec![0xAB; 3_000_000];
    let sealed = seal(&inbox_pub, &photo);
    assert_eq!(sealed.len(), 32 + photo.len() + 16, "48 bytes of overhead");
    assert_eq!(open(&inbox_secret, &sealed).as_deref(), Some(&photo[..]));

    // The page can seal but not open: the public key does not open it, a different inbox does not, a flipped bit does not.
    assert!(open(&inbox_pub, &sealed).is_none());
    let mut other = [0u8; 32];
    getrandom::fill(&mut other).unwrap();
    assert!(open(&other, &sealed).is_none());
    let mut bad = sealed.clone();
    bad[100] ^= 1;
    assert!(open(&inbox_secret, &bad).is_none());
    // Two uploads of the same file differ (fresh ephemeral key), so the service cannot tell a repeat.
    assert_ne!(seal(&inbox_pub, &photo)[..64], sealed[..64]);
}
