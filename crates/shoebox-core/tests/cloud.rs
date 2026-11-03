//! W1-02: blob encryption for Shoebox Cloud, and the cloud secret that comes from the recovery kit.

use shoebox_core::cloud::{
    decrypt_blob, decrypt_stream, encrypt_blob, encrypt_stream, BlobKey, CloudCryptoError,
    CloudSecret, CHUNK,
};
use shoebox_core::identity::KeyPair;
use shoebox_core::ids::Blake3Hash;
use shoebox_core::recovery::RecoveryKit;
use shoebox_core::store::LibraryId;
use std::io::{self, Read, Write};

fn secret(n: u8) -> CloudSecret {
    CloudSecret::from_root(&KeyPair::from_seed([n; 32]))
}

fn bytes(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 31 + i / 251) as u8).collect()
}

#[test]
fn the_same_plaintext_gives_the_same_cid_on_two_devices_and_another_secret_gives_another() {
    let data = bytes(100_000);
    let a = encrypt_blob(&secret(1), &data);
    let b = encrypt_blob(&secret(1), &data);
    assert_eq!(a.cid, b.cid);
    assert_eq!(a.ciphertext, b.ciphertext);
    assert_eq!(a.key, b.key);
    let c = encrypt_blob(&secret(2), &data);
    assert_ne!(a.cid, c.cid);
    assert_ne!(a.key, c.key);
    // Different content under the same secret has its own key.
    assert_ne!(a.key, encrypt_blob(&secret(1), &bytes(100_001)).key);
}

#[test]
fn round_trips_at_every_piece_boundary() {
    let s = secret(3);
    for len in [
        0,
        1,
        15,
        16,
        CHUNK - 1,
        CHUNK,
        CHUNK + 1,
        2 * CHUNK,
        2 * CHUNK + 7,
    ] {
        let data = bytes(len);
        let sealed = encrypt_blob(&s, &data);
        let pieces = len.div_ceil(CHUNK).max(1);
        assert_eq!(sealed.ciphertext.len(), len + 16 * pieces, "length {len}");
        assert_eq!(sealed.cid, Blake3Hash::of_bytes(&sealed.ciphertext));
        assert_eq!(
            decrypt_blob(&sealed.key, &sealed.ciphertext).unwrap(),
            data,
            "length {len}"
        );
    }
}

#[test]
fn a_damaged_blob_a_wrong_key_and_a_cut_blob_do_not_open() {
    let s = secret(4);
    let data = bytes(2 * CHUNK + 100);
    let sealed = encrypt_blob(&s, &data);

    let mut flipped = sealed.ciphertext.clone();
    flipped[CHUNK + 20] ^= 1;
    assert!(matches!(
        decrypt_blob(&sealed.key, &flipped),
        Err(CloudCryptoError::BadCiphertext)
    ));

    let wrong = encrypt_blob(&s, b"something else").key;
    assert!(decrypt_blob(&wrong, &sealed.ciphertext).is_err());

    // Cut at a piece boundary: the last piece left was not sealed as the last, so it fails.
    let piece = CHUNK + 16;
    assert!(decrypt_blob(&sealed.key, &sealed.ciphertext[..piece]).is_err());
    assert!(decrypt_blob(&sealed.key, &sealed.ciphertext[..2 * piece]).is_err());
    // Cut mid-piece, and extended with a piece of garbage.
    assert!(decrypt_blob(
        &sealed.key,
        &sealed.ciphertext[..sealed.ciphertext.len() - 1]
    )
    .is_err());
    let mut longer = sealed.ciphertext.clone();
    longer.extend_from_slice(&[0u8; 16]);
    assert!(decrypt_blob(&sealed.key, &longer).is_err());
    // Empty input is not a blob (even an empty plaintext has a sealed final piece).
    assert!(decrypt_blob(&sealed.key, &[]).is_err());
}

#[test]
fn a_key_opens_only_its_own_blob() {
    let s = secret(5);
    let one = encrypt_blob(&s, &bytes(1000));
    let two = encrypt_blob(&s, &bytes(2000));
    assert!(decrypt_blob(&one.key, &two.ciphertext).is_err());
}

#[test]
fn decrypting_reports_the_plaintext_hash_so_a_restore_can_check_it() {
    let s = secret(6);
    let data = bytes(CHUNK + 5);
    let hash = Blake3Hash::of_bytes(&data);
    let key = s.blob_key(&hash);
    let mut sealed = Vec::new();
    encrypt_stream(&key, &data[..], &mut sealed).unwrap();
    let mut out = Vec::new();
    let got = decrypt_stream(&key, &sealed[..], &mut out).unwrap();
    assert_eq!(got, hash);
    assert_eq!(out, data);
}

#[test]
fn a_restored_library_derives_the_same_secret_from_its_kit() {
    let kit = RecoveryKit::new(LibraryId([9; 16]), 0, KeyPair::from_seed([7; 32]));
    let from_text = RecoveryKit::from_text(&kit.to_text()).unwrap();
    assert_eq!(kit.cloud_secret(), from_text.cloud_secret());
    let root = RecoveryKit::root_from_phrase(&kit.phrase()).unwrap();
    assert_eq!(CloudSecret::from_root(&root), kit.cloud_secret());
    // The secret is not the root seed or its public key.
    assert_ne!(kit.cloud_secret().as_bytes(), &kit.root().seed());
    assert_ne!(kit.cloud_secret().as_bytes(), &kit.root().public_bytes());
    // And it never prints.
    assert!(
        !format!("{:?}", kit.cloud_secret()).contains(&hex::encode(kit.cloud_secret().as_bytes()))
    );
}

/// Locks the format: if this changes, every blob already in a cloud stops opening.
#[test]
fn golden_bytes_lock_the_format() {
    let s = CloudSecret::from_bytes([0x11; 32]);
    let data = b"shoebox cloud golden blob".to_vec();
    let sealed = encrypt_blob(&s, &data);
    assert_eq!(
        hex::encode(sealed.key.0),
        "87556a3403b9edae0de5e7c25a21bb1a75cec4af410de278e8846a42e1a89ed5",
        "blob key"
    );
    assert_eq!(
        hex::encode(sealed.cid.0),
        "a889390d04cc3e223e2d3a8ebb84a60217dba3567bbd7fe2fc8360303f339c1d",
        "cid"
    );
    assert_eq!(
        hex::encode(&sealed.ciphertext),
        "ad4dbadb005cb1cb873ade410d8663c3a846e4f9447f46ee0cec0ef6a381ecb27ee77bbda6cc5934bc",
        "ciphertext"
    );
    assert_eq!(
        hex::encode(CloudSecret::from_root(&KeyPair::from_seed([0; 32])).as_bytes()),
        "7b39849cbd445620716ed03e693e9507150ed4b270146488878a5e96e0953eb2",
        "secret from the all-zero root"
    );
}

/// A reader of `left` bytes whose content depends on position only, not on how it is read.
struct Pattern {
    pos: u64,
    len: u64,
}
impl Read for Pattern {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = (buf.len() as u64).min(self.len - self.pos) as usize;
        for (i, b) in buf[..n].iter_mut().enumerate() {
            *b = ((self.pos + i as u64) % 251) as u8;
        }
        self.pos += n as u64;
        Ok(n)
    }
}
fn pattern(len: u64) -> Pattern {
    Pattern { pos: 0, len }
}
struct HashSink(blake3::Hasher, u64);
impl Write for HashSink {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0.update(b);
        self.1 += b.len() as u64;
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Seal a generated stream into a file, open it again, and compare with the stream's own hash.
fn stream_round_trip(len: u64) {
    let key = BlobKey([42; 32]);
    let path =
        std::env::temp_dir().join(format!("shoebox-cloud-test-{}-{len}", std::process::id()));
    let file = std::fs::File::create(&path).unwrap();
    let (cid, written) = encrypt_stream(&key, pattern(len), io::BufWriter::new(file)).unwrap();
    let on_disk = std::fs::metadata(&path).unwrap().len();
    assert_eq!(written, on_disk);
    assert_eq!(on_disk, len + 16 * len.div_ceil(CHUNK as u64));

    let mut expected = HashSink(blake3::Hasher::new(), 0);
    io::copy(&mut pattern(len), &mut expected).unwrap();
    let mut opened = HashSink(blake3::Hasher::new(), 0);
    let reader = io::BufReader::new(std::fs::File::open(&path).unwrap());
    let hash = decrypt_stream(&key, reader, &mut opened).unwrap();
    let mut file_hash = HashSink(blake3::Hasher::new(), 0);
    io::copy(&mut std::fs::File::open(&path).unwrap(), &mut file_hash).unwrap();
    std::fs::remove_file(&path).unwrap();

    assert_eq!(opened.1, len);
    assert_eq!(
        *opened.0.finalize().as_bytes(),
        *expected.0.finalize().as_bytes()
    );
    assert_eq!(hash.0, *opened.0.finalize().as_bytes());
    assert_eq!(cid.0, *file_hash.0.finalize().as_bytes());
}

#[test]
fn a_stream_of_64_mib_round_trips_through_a_file() {
    stream_round_trip(64 * 1024 * 1024 + 3);
}

/// `cargo test -p shoebox-core --release --test cloud -- --ignored`. Memory is one piece and a byte, by construction.
#[test]
#[ignore = "1 GiB: slow in a debug build"]
fn a_stream_of_1_gib_round_trips() {
    stream_round_trip(1024 * 1024 * 1024);
}

mod auth {
    use shoebox_core::cloud_auth::{AuthError, CloudAuth};
    use shoebox_core::identity::{issue_device_cert, KeyPair};
    use shoebox_core::model::Role;
    use shoebox_core::store::LibraryId;
    use shoebox_core::Hlc;

    const LIB: LibraryId = LibraryId([3; 16]);

    fn hlc() -> Hlc {
        Hlc {
            physical_ms: 1,
            counter: 0,
            device: shoebox_core::EndpointId([0; 32]),
        }
    }

    fn signed(method: &str, path: &str, body: &[u8], now: i64) -> (CloudAuth, KeyPair) {
        let root = KeyPair::from_seed([1; 32]);
        let device = KeyPair::from_seed([2; 32]);
        let cert = issue_device_cert(
            &root,
            LIB,
            device.endpoint_id(),
            Role::Keeper,
            "nest",
            hlc(),
            false,
        );
        (
            CloudAuth::sign(
                &device,
                LIB,
                root.root_public(),
                None,
                cert,
                method,
                path,
                now,
                body,
            ),
            device,
        )
    }

    #[test]
    fn a_signed_request_verifies_and_survives_the_header() {
        let (a, _) = signed("PUT", "/v1/blobs/ab", b"body", 1_000_000);
        let back = CloudAuth::from_header(&a.to_header()).unwrap();
        assert_eq!(back, a);
        assert!(back
            .verify("PUT", "/v1/blobs/ab", b"body", 1_000_500, 60_000)
            .is_ok());
    }

    #[test]
    fn another_method_path_body_or_time_does_not_verify() {
        let (a, _) = signed("PUT", "/v1/blobs/ab", b"body", 1_000_000);
        let now = 1_000_000;
        assert_eq!(
            a.verify("DELETE", "/v1/blobs/ab", b"body", now, 60_000),
            Err(AuthError::BadSignature)
        );
        assert_eq!(
            a.verify("PUT", "/v1/blobs/ac", b"body", now, 60_000),
            Err(AuthError::BadSignature)
        );
        assert_eq!(
            a.verify("PUT", "/v1/blobs/ab", b"BODY", now, 60_000),
            Err(AuthError::BadSignature)
        );
        assert_eq!(
            a.verify("PUT", "/v1/blobs/ab", b"body", now + 120_000, 60_000),
            Err(AuthError::Stale)
        );
        assert_eq!(
            a.verify("PUT", "/v1/blobs/ab", b"body", now - 120_000, 60_000),
            Err(AuthError::Stale)
        );
    }

    #[test]
    fn a_signature_by_another_key_than_the_certified_device_fails() {
        let (mut a, _) = signed("GET", "/v1/usage", b"", 5);
        let other = KeyPair::from_seed([9; 32]);
        a.signature = other.sign(b"anything");
        assert_eq!(
            a.verify("GET", "/v1/usage", b"", 5, 1),
            Err(AuthError::BadSignature)
        );
    }

    #[test]
    fn garbage_headers_are_malformed() {
        assert_eq!(CloudAuth::from_header("zz"), Err(AuthError::Malformed));
        assert_eq!(CloudAuth::from_header("00ff"), Err(AuthError::Malformed));
        assert_eq!(CloudAuth::from_header(""), Err(AuthError::Malformed));
    }
}
