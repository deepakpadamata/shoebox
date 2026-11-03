//! W1-03: the share manifest: format, signing, golden bytes.

use shoebox_core::identity::KeyPair;
use shoebox_core::ids::{Blake3Hash, Timestamp};
use shoebox_core::share::{
    open, read_header, seal, seal_with_nonce, CloudBlob, Item, ItemKind, Manifest, ShareError,
    ShareKey, FORMAT,
};
use std::time::Instant;

fn blob(n: u32) -> CloudBlob {
    let mut cid = [0u8; 32];
    cid[..4].copy_from_slice(&n.to_le_bytes());
    CloudBlob {
        cid: Blake3Hash(cid),
        key: [(n % 251) as u8; 32],
        size: 10_000 + u64::from(n),
    }
}

fn item(n: u32) -> Item {
    Item {
        kind: if n.is_multiple_of(17) {
            ItemKind::Video
        } else {
            ItemKind::Photo
        },
        taken: Timestamp(1_600_000_000_000 + i64::from(n) * 61_000),
        width: 4000,
        height: 3000,
        orientation: 1 + (n % 8) as u8,
        thumb: blob(n * 4),
        preview: blob(n * 4 + 1),
        original: (n.is_multiple_of(3)).then(|| blob(n * 4 + 2)),
        motion: (n.is_multiple_of(5)).then(|| blob(n * 4 + 3)),
    }
}

fn manifest(items: u32) -> Manifest {
    Manifest {
        album_name: "Lisbon, spring".into(),
        items: (0..items).map(item).collect(),
        expires: Some(Timestamp(1_900_000_000_000)),
        inbox_key: None,
    }
}

fn signer() -> KeyPair {
    KeyPair::from_seed([3; 32])
}

const ID: [u8; 16] = [7; 16];

#[test]
fn a_manifest_round_trips_with_its_header() {
    let key = ShareKey::generate();
    let m = manifest(50);
    let sealed = seal(&m, ID, 4, &key, &signer());
    let header = read_header(&sealed).unwrap();
    assert_eq!((header.share_id, header.manifest_version), (ID, 4));
    let (h, back) = open(&sealed, &key, &signer().public_bytes()).unwrap();
    assert_eq!(h, header);
    assert_eq!(back, m);
}

#[test]
fn two_seals_of_the_same_manifest_differ() {
    let key = ShareKey::generate();
    let m = manifest(3);
    assert_ne!(
        seal(&m, ID, 1, &key, &signer()),
        seal(&m, ID, 1, &key, &signer())
    );
}

#[test]
fn a_wrong_signature_a_wrong_key_and_a_cut_body_are_told_apart() {
    let key = ShareKey::generate();
    let sealed = seal(&manifest(20), ID, 1, &key, &signer());
    let who = signer().public_bytes();

    // Someone else's signing key.
    let other = KeyPair::from_seed([4; 32]).public_bytes();
    assert_eq!(
        open(&sealed, &key, &other).unwrap_err(),
        ShareError::BadSignature
    );

    // The right sender, a key that does not open it.
    assert_eq!(
        open(&sealed, &ShareKey::generate(), &who).unwrap_err(),
        ShareError::WrongKey
    );

    // A flipped byte anywhere in the body, header or signature breaks the signature.
    for at in [0usize, 5, 8, 22, 30, 60, sealed.len() / 2, sealed.len() - 1] {
        let mut bad = sealed.clone();
        bad[at] ^= 1;
        let err = open(&bad, &key, &who).unwrap_err();
        assert!(
            matches!(err, ShareError::BadSignature | ShareError::NotAShare),
            "byte {at}: {err:?}"
        );
    }

    // Cut short, at several places; extended.
    for len in [5, 40, 60, 200, sealed.len() - 1] {
        assert_eq!(
            open(&sealed[..len], &key, &who).unwrap_err(),
            ShareError::Truncated,
            "len {len}"
        );
    }
    let mut longer = sealed.clone();
    longer.push(0);
    assert_eq!(
        open(&longer, &key, &who).unwrap_err(),
        ShareError::Truncated
    );

    // Not a share at all.
    assert_eq!(
        open(b"hello, world, this is not it", &key, &who).unwrap_err(),
        ShareError::NotAShare
    );
    assert_eq!(open(&[], &key, &who).unwrap_err(), ShareError::NotAShare);
}

#[test]
fn a_header_from_another_manifest_cannot_be_swapped_in() {
    // The server may not graft one manifest's ciphertext under another share id or version: both are
    // signed and bound into the encryption.
    let key = ShareKey::generate();
    let a = seal(&manifest(5), ID, 1, &key, &signer());
    let mut forged = a.clone();
    forged[21] = 9; // manifest version 1 -> 9
    assert_eq!(
        open(&forged, &key, &signer().public_bytes()).unwrap_err(),
        ShareError::BadSignature
    );
}

#[test]
fn a_newer_format_says_to_update_the_page() {
    let key = ShareKey::generate();
    let mut sealed = seal(&manifest(2), ID, 1, &key, &signer());
    sealed[4] = FORMAT + 1;
    assert_eq!(
        open(&sealed, &key, &signer().public_bytes()).unwrap_err(),
        ShareError::NewerFormat { found: FORMAT + 1 }
    );
    // Even when the rest of a newer layout is unrecognisable.
    let short = &sealed[..9];
    assert_eq!(
        open(short, &key, &signer().public_bytes()).unwrap_err(),
        ShareError::NewerFormat { found: FORMAT + 1 }
    );
}

#[test]
fn the_reserved_inbox_key_round_trips() {
    let key = ShareKey::generate();
    let mut m = manifest(1);
    m.inbox_key = Some([9; 32]);
    let (_, back) = open(
        &seal(&m, ID, 1, &key, &signer()),
        &key,
        &signer().public_bytes(),
    )
    .unwrap();
    assert_eq!(back.inbox_key, Some([9; 32]));
}

#[test]
fn secrets_do_not_print() {
    assert!(!format!("{:?}", ShareKey([0xAB; 32])).contains("abab"));
    assert!(!format!("{:?}", blob(1).key()).contains("0101"));
}

/// Locks the format: if this changes, every share already published stops opening.
#[test]
fn golden_bytes_lock_the_format() {
    let key = ShareKey([0x22; 32]);
    let m = manifest(2);
    let sealed = seal_with_nonce(&m, ID, 3, &key, &signer(), [0x33; 24]);
    assert_eq!(
        hex::encode(&sealed),
        "53425853010707070707070707070707070707070703000000333333333333333333333333333333333333333333333333d0010000612c17434b264e781a44243da9c6c543523faca96fb4d6a9a373961eabc0aad0e1dec1495b82b1c4f0bf8fc698f6b6f20552d8994a5261a7b7ff7f808bb1f15bb4f8aac82bb7604761304fb032a0be2ade2d6d07261f56992cc919f48731ffde995997348c4d90d88bd504b721fa0b395820383e5b430a35430ecb511912f6bcb854cc852dc48a670782952c02acc298ed4ab02009af3f96fe0226238aecef3d92bc6a817be498e8c5c0fe38037e60494bf464cfc6d565df8fdadf5716b5d81ef80eed0e863b9b8fc9804ae7909a2c7008815d747d975452acb004521baeb7cfd206c2b25a11803faf974f0bd2815fc9ab9e85ef9684ffac6835f4af8c524f47d699875da539d914825af399299c371e8f9fa55a3dd1dde88789b16f057a7ca081de2273aa7ac20607166d2a86c81a34152fce062681811c52974ace6831cd89f039624b047a7e22fd21d0a1629119b9a500af062c7eb5ed762101084ef97cb0e314e476a6e94fc6b3cfa9840c1fe0135cd3d9d0f1252babad56ca2d7aff91c90ceb753b5e7c9e337da8aa51508220afe4e781de30f76e83a04552ed21a2581331ad52c452f38b9515790628a1f47ed49311dce964e943967f4c496765f657c68925076faa57ee02747384ae4c69ae0367db600a33a074d55214ee7846b2af5ebeadd93cd9b9fb1c2961936dbf0d1bd123f2c1f88d5bf3b23786ffd91339fe26ab5747fcefc1b402d636c1fcaf86ca0c"
    );
    assert!(open(&sealed, &key, &signer().public_bytes()).is_ok());
}

#[test]
fn fifty_thousand_items_stay_small_and_fast() {
    let key = ShareKey::generate();
    let m = manifest(50_000);
    let t = Instant::now();
    let sealed = seal(&m, ID, 1, &key, &signer());
    let encode = t.elapsed();
    let t = Instant::now();
    let (_, back) = open(&sealed, &key, &signer().public_bytes()).unwrap();
    let decode = t.elapsed();
    assert_eq!(back.items.len(), 50_000);
    eprintln!(
        "50k items: {} bytes, encode {encode:?}, decode {decode:?} (debug build)",
        sealed.len()
    );
    assert!(sealed.len() < 10 * 1024 * 1024, "{} bytes", sealed.len());
    // Generous in a debug build; the release numbers are far under the issue's 1 s and 500 ms.
    assert!(encode.as_secs_f32() < 5.0 && decode.as_secs_f32() < 5.0);
}

mod link {
    use shoebox_core::share::{LinkMode, ShareKey, ShareLink};

    #[test]
    fn a_cloud_link_round_trips_through_its_fragment() {
        let link = ShareLink::cloud([0xAB; 16], ShareKey([0x5C; 32]));
        let text = link.to_fragment();
        assert!(text.starts_with("c."));
        assert!(text.len() < 80, "{text}");
        assert!(text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b)));
        let back = ShareLink::parse(&format!("#{text}")).unwrap();
        assert_eq!(back, link);
        assert_eq!(back.mode, LinkMode::Cloud);
        assert_eq!(back.share_id(), Some([0xAB; 16]));
    }

    #[test]
    fn every_length_of_bytes_round_trips_and_garbage_is_refused() {
        for n in 1..=40usize {
            let mut l = ShareLink::cloud([1; 16], ShareKey([2; 32]));
            l.mode = LinkMode::Live;
            l.locator = (0..n as u8).collect();
            assert_eq!(ShareLink::parse(&l.to_fragment()).unwrap(), l, "{n} bytes");
        }
        let good = ShareLink::cloud([9; 16], ShareKey([8; 32])).to_fragment();
        for bad in [
            "".to_string(),
            "x.AAAA.AAAA".to_string(),
            "c.AAAA".to_string(),
            format!("{good}.extra"),
            good.replace('.', "!"),
            "c.AAAA.AAAA".to_string(), // locator too short for a cloud link
            good.replace("c.", "c.====").to_string(),
            format!("{}B", &good[..good.len() - 1]), // non-canonical trailing bits
        ] {
            assert!(ShareLink::parse(&bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn the_signing_key_comes_from_the_share_key_alone() {
        let k = ShareKey([3; 32]);
        assert_eq!(
            k.signer().public_bytes(),
            ShareKey([3; 32]).signer().public_bytes()
        );
        assert_ne!(
            k.signer().public_bytes(),
            ShareKey([4; 32]).signer().public_bytes()
        );
        assert_ne!(k.signer().seed(), k.0);
    }
}
