//! Blob encryption for Shoebox Cloud (W1-02). Everything the cloud holds is ciphertext; a restore
//! from the recovery kit alone can decrypt it.
//!
//! * `CloudSecret` is derived from the library root key, so the 24 words are all a restore needs.
//! * A blob's key is `k_b = KDF(secret, hash of the plaintext)`: it depends on the content, so the same
//!   photo encrypts to the same bytes on every device, and handing out `k_b` exposes that one blob only.
//! * The ciphertext is cut into 4 MiB pieces, each sealed with XChaCha20-Poly1305 under a nonce derived
//!   from `k_b`, the piece number and whether it is the last (so cutting a blob short or extending it
//!   fails to open). A phone never holds more than a couple of pieces.
//! * A blob is stored under `cid = blake3(ciphertext)`.

use crate::identity::KeyPair;
use crate::ids::Blake3Hash;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use std::fmt;
use std::io::{self, Read, Write};

/// Plaintext bytes per sealed piece (matches the fetcher's 4 MiB pieces).
pub const CHUNK: usize = 4 * 1024 * 1024;
/// Bytes a seal adds to each piece.
pub const TAG: usize = 16;

const SECRET_CONTEXT: &str = "shoebox/cloud-secret/v1";
const BLOB_KEY_CONTEXT: &str = "shoebox/cloud-blob-key/v1";

/// The content id of a stored blob: the BLAKE3 hash of its ciphertext.
pub type Cid = Blake3Hash;

#[derive(Debug, thiserror::Error)]
pub enum CloudCryptoError {
    #[error("the blob did not open: it is damaged, cut short, or the key is wrong")]
    BadCiphertext,
    #[error("i/o error: {0}")]
    Io(#[from] io::Error),
}

/// A library's cloud secret. Never printed; derive it again from the root key when needed.
#[derive(Clone, PartialEq, Eq)]
pub struct CloudSecret([u8; 32]);

impl CloudSecret {
    /// Derived from the library root key (domain separated), so a library created before the cloud existed
    /// gets its secret on first use and the recovery phrase does not change.
    pub fn from_root(root: &KeyPair) -> Self {
        Self(blake3::derive_key(SECRET_CONTEXT, &root.seed()))
    }
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
    /// The key of the blob whose plaintext hashes to `plaintext`.
    pub fn blob_key(&self, plaintext: &Blake3Hash) -> BlobKey {
        let mut h = blake3::Hasher::new_derive_key(BLOB_KEY_CONTEXT);
        h.update(&self.0);
        h.update(&plaintext.0);
        BlobKey(*h.finalize().as_bytes())
    }
}

impl fmt::Debug for CloudSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CloudSecret(..)")
    }
}

/// The key of one blob (`k_b`). It travels in a share manifest; it opens that blob only.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BlobKey(pub [u8; 32]);

impl fmt::Debug for BlobKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BlobKey(..)")
    }
}

impl BlobKey {
    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new((&self.0).into())
    }
    /// Seal one piece of a blob: `index` counts from zero, `last` is true for the final piece (which may be short,
    /// and is the only piece of a blob of `CHUNK` bytes or fewer). [`encrypt_stream`] is this in a loop; an async
    /// caller reading a blob piece by piece uses it directly.
    pub fn seal_piece(&self, index: u64, last: bool, plain: &[u8]) -> Vec<u8> {
        self.cipher()
            .encrypt(&self.nonce(index, last), plain)
            .expect("sealing a piece cannot fail")
    }

    /// Open one sealed piece (`CHUNK` bytes plus a 16-byte tag, or fewer for the last piece). A viewer that plays
    /// a video seeks to a piece, fetches it by byte range and opens it alone.
    pub fn open_piece(
        &self,
        index: u64,
        last: bool,
        sealed: &[u8],
    ) -> Result<Vec<u8>, CloudCryptoError> {
        self.cipher()
            .decrypt(&self.nonce(index, last), sealed)
            .map_err(|_| CloudCryptoError::BadCiphertext)
    }
    fn nonce(&self, index: u64, last: bool) -> XNonce {
        let mut h = blake3::Hasher::new_keyed(&self.0);
        h.update(b"shoebox/cloud-nonce/v1");
        h.update(&index.to_le_bytes());
        h.update(&[u8::from(last)]);
        let mut n = [0u8; 24];
        h.finalize_xof().fill(&mut n);
        n.into()
    }
}

/// What [`encrypt_blob`] returns.
#[derive(Debug, Clone)]
pub struct Sealed {
    pub cid: Cid,
    pub key: BlobKey,
    pub ciphertext: Vec<u8>,
}

/// Seal a whole blob held in memory (small blobs, tests); its key comes from the hash of `plaintext`.
pub fn encrypt_blob(secret: &CloudSecret, plaintext: &[u8]) -> Sealed {
    let key = secret.blob_key(&Blake3Hash::of_bytes(plaintext));
    let mut out = Vec::with_capacity(plaintext.len() + TAG * (plaintext.len() / CHUNK + 1));
    let (cid, _) = encrypt_stream(&key, plaintext, &mut out).expect("memory i/o cannot fail");
    Sealed {
        cid,
        key,
        ciphertext: out,
    }
}

/// Open a whole blob held in memory.
pub fn decrypt_blob(key: &BlobKey, ciphertext: &[u8]) -> Result<Vec<u8>, CloudCryptoError> {
    let mut out = Vec::with_capacity(ciphertext.len());
    decrypt_stream(key, ciphertext, &mut out)?;
    Ok(out)
}

/// Fill `buf` from `r` as far as it will go; fewer bytes than `buf.len()` means the end.
fn read_full(r: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

/// Seal a stream piece by piece. Returns the `cid` of everything written and the ciphertext length.
/// Memory use is one piece plus one byte of lookahead.
pub fn encrypt_stream(
    key: &BlobKey,
    mut plain: impl Read,
    mut out: impl Write,
) -> io::Result<(Cid, u64)> {
    let mut hasher = blake3::Hasher::new();
    let mut written = 0u64;
    let mut buf = vec![0u8; CHUNK + 1];
    let mut have = read_full(&mut plain, &mut buf)?;
    let mut index = 0u64;
    loop {
        let last = have <= CHUNK;
        let take = have.min(CHUNK);
        let sealed = key.seal_piece(index, last, &buf[..take]);
        hasher.update(&sealed);
        out.write_all(&sealed)?;
        written += sealed.len() as u64;
        if last {
            break;
        }
        buf[0] = buf[CHUNK];
        have = 1 + read_full(&mut plain, &mut buf[1..])?;
        index += 1;
    }
    Ok((Blake3Hash(*hasher.finalize().as_bytes()), written))
}

/// Open a stream piece by piece. Returns the BLAKE3 hash of the plaintext, so a caller that knows the blob's
/// library hash (a restore) can check it; a viewer, which has only the key, relies on the seals.
pub fn decrypt_stream(
    key: &BlobKey,
    mut sealed: impl Read,
    mut out: impl Write,
) -> Result<Blake3Hash, CloudCryptoError> {
    let mut hasher = blake3::Hasher::new();
    let piece = CHUNK + TAG;
    let mut buf = vec![0u8; piece + 1];
    let mut have = read_full(&mut sealed, &mut buf)?;
    let mut index = 0u64;
    loop {
        let last = have <= piece;
        let take = have.min(piece);
        if take < TAG {
            return Err(CloudCryptoError::BadCiphertext);
        }
        let plain = key.open_piece(index, last, &buf[..take])?;
        hasher.update(&plain);
        out.write_all(&plain)?;
        if last {
            break;
        }
        buf[0] = buf[piece];
        have = 1 + read_full(&mut sealed, &mut buf[1..])?;
        index += 1;
    }
    Ok(Blake3Hash(*hasher.finalize().as_bytes()))
}
