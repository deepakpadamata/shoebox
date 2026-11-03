//! The browser side of a Shoebox web link (W1-01, W1-06): opens a share manifest and decrypts blobs, using
//! the same code the apps use, so the format has one implementation.
//!
//! Built for `wasm32-unknown-unknown` and wrapped by `wasm-bindgen`; the plain Rust functions are tested
//! natively.

use shoebox_core::cloud::{decrypt_blob as core_decrypt, BlobKey, CHUNK, TAG};
use shoebox_core::share::{self, CloudBlob, Item, ItemKind, LinkMode, ShareKey, ShareLink};
use wasm_bindgen::prelude::*;

/// What went wrong, as a short code the page maps to a message, then a human sentence.
fn fail(code: &str, e: impl std::fmt::Display) -> JsError {
    JsError::new(&format!("{code}: {e}"))
}

fn key32(bytes: &[u8], what: &str) -> Result<[u8; 32], JsError> {
    bytes
        .try_into()
        .map_err(|_| fail("bad-key", format!("{what} must be 32 bytes")))
}

/// Piece sizes, so the page can turn a plaintext offset into a byte range of the ciphertext.
#[wasm_bindgen]
pub fn piece_size() -> usize {
    CHUNK + TAG
}

/// Decrypt a whole blob (a thumbnail or preview). Fails if it is damaged, cut short, or the key is wrong.
#[wasm_bindgen]
pub fn decrypt_blob(key: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, JsError> {
    let key = BlobKey(key32(key, "a blob key")?);
    core_decrypt(&key, ciphertext).map_err(|e| fail("bad-blob", e))
}

/// Decrypt one piece of a blob, for playing a video without holding it whole.
#[wasm_bindgen]
pub fn decrypt_piece(
    key: &[u8],
    index: u32,
    last: bool,
    sealed: &[u8],
) -> Result<Vec<u8>, JsError> {
    let key = BlobKey(key32(key, "a blob key")?);
    key.open_piece(u64::from(index), last, sealed)
        .map_err(|e| fail("bad-blob", e))
}

/// The content id of some ciphertext (BLAKE3), as lower-case hex, to check a download against its id.
#[wasm_bindgen]
pub fn cid_of(ciphertext: &[u8]) -> String {
    hex::encode(blake3::hash(ciphertext).as_bytes())
}

fn blob_json(b: &CloudBlob) -> serde_json::Value {
    serde_json::json!({ "cid": hex::encode(b.cid.0), "key": hex::encode(b.key), "size": b.size })
}

fn item_json(i: &Item) -> serde_json::Value {
    serde_json::json!({
        "kind": match i.kind { ItemKind::Photo => "photo", ItemKind::Video => "video" },
        "taken": i.taken.0,
        "width": i.width,
        "height": i.height,
        "orientation": i.orientation,
        "thumb": blob_json(&i.thumb),
        "preview": blob_json(&i.preview),
        "original": i.original.as_ref().map(blob_json),
        "motion": i.motion.as_ref().map(blob_json),
    })
}

/// Read the text after a link's `#`. Returns JSON `{ mode: "cloud" | "live", locator: <hex>, key: <hex> }`, or
/// fails with `bad-link` when the text is not a Shoebox link (a cut-off copy, say).
#[wasm_bindgen]
pub fn parse_link(fragment: &str) -> Result<String, JsError> {
    let link = ShareLink::parse(fragment)
        .ok_or_else(|| fail("bad-link", "this is not a complete Shoebox link"))?;
    Ok(serde_json::json!({
        "mode": match link.mode { LinkMode::Cloud => "cloud", LinkMode::Live => "live" },
        "locator": hex::encode(&link.locator),
        "key": hex::encode(link.key.0),
    })
    .to_string())
}

/// [`open_manifest`] for a link: the signing key is derived from the share key, so the link is all it needs.
#[wasm_bindgen]
pub fn open_share(sealed: &[u8], share_key: &[u8]) -> Result<String, JsError> {
    let key = ShareKey(key32(share_key, "the share key")?);
    open_manifest(sealed, share_key, &key.signer().public_bytes())
}

/// Check, decrypt and decode a sealed manifest. Returns JSON:
/// `{ shareId, version, album, expires, items: [...] }`, with blob ids and keys as hex.
/// The error text starts with one of `not-a-share`, `truncated`, `newer-format`, `bad-signature`,
/// `wrong-key`, `malformed`.
#[wasm_bindgen]
pub fn open_manifest(sealed: &[u8], share_key: &[u8], signer: &[u8]) -> Result<String, JsError> {
    use share::ShareError::*;
    let key = ShareKey(key32(share_key, "the share key")?);
    let signer = key32(signer, "the sender's key")?;
    let (header, m) = share::open(sealed, &key, &signer).map_err(|e| {
        let code = match e {
            NotAShare => "not-a-share",
            Truncated => "truncated",
            NewerFormat { .. } => "newer-format",
            BadSignature => "bad-signature",
            WrongKey => "wrong-key",
            Malformed => "malformed",
        };
        fail(code, e)
    })?;
    let json = serde_json::json!({
        "shareId": hex::encode(header.share_id),
        "version": header.manifest_version,
        "album": m.album_name,
        "expires": m.expires.map(|t| t.0),
        "items": m.items.iter().map(item_json).collect::<Vec<_>>(),
    });
    Ok(json.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shoebox_core::cloud::{encrypt_blob, CloudSecret};
    use shoebox_core::identity::KeyPair;
    use shoebox_core::ids::{Blake3Hash, Timestamp};
    use shoebox_core::share::{seal, Manifest, ShareKey};

    #[test]
    fn decrypts_a_blob_and_names_its_cid() {
        let sealed = encrypt_blob(&CloudSecret::from_bytes([5; 32]), b"a small photo");
        assert_eq!(cid_of(&sealed.ciphertext), hex::encode(sealed.cid.0));
        let plain = decrypt_blob(&sealed.key.0, &sealed.ciphertext)
            .ok()
            .unwrap();
        assert_eq!(plain, b"a small photo");
        let piece = decrypt_piece(&sealed.key.0, 0, true, &sealed.ciphertext)
            .ok()
            .unwrap();
        assert_eq!(piece, b"a small photo");
    }

    #[test]
    fn a_link_parses_and_its_key_opens_its_share() {
        let key = ShareKey([0x31; 32]);
        let link = ShareLink::cloud([4; 16], key.clone());
        let v: serde_json::Value =
            serde_json::from_str(&parse_link(&link.to_fragment()).ok().unwrap()).unwrap();
        assert_eq!(v["mode"], "cloud");
        assert_eq!(v["locator"], hex::encode([4u8; 16]));
        assert_eq!(v["key"], hex::encode([0x31u8; 32]));
        let m = Manifest {
            album_name: "A".into(),
            items: vec![],
            expires: None,
            inbox_key: None,
        };
        let sealed = seal(&m, [4; 16], 1, &key, &key.signer());
        let json = open_share(&sealed, &key.0).ok().unwrap();
        assert!(json.contains("\"album\":\"A\""));
    }

    #[test]
    fn opens_a_manifest_into_json_for_the_page() {
        let blob = |n: u8| CloudBlob {
            cid: Blake3Hash([n; 32]),
            key: [n; 32],
            size: 7,
        };
        let manifest = Manifest {
            album_name: "Trip".into(),
            items: vec![Item {
                kind: ItemKind::Video,
                taken: Timestamp(1_700_000_000_000),
                width: 640,
                height: 480,
                orientation: 6,
                thumb: blob(1),
                preview: blob(2),
                original: None,
                motion: Some(blob(3)),
            }],
            expires: None,
            inbox_key: None,
        };
        let key = ShareKey([9; 32]);
        let signer = KeyPair::from_seed([1; 32]);
        let sealed = seal(&manifest, [4; 16], 2, &key, &signer);
        let json = open_manifest(&sealed, &key.0, &signer.public_bytes())
            .ok()
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["album"], "Trip");
        assert_eq!(v["version"], 2);
        assert_eq!(v["shareId"], hex::encode([4u8; 16]));
        assert_eq!(v["items"][0]["kind"], "video");
        assert_eq!(v["items"][0]["motion"]["key"], hex::encode([3u8; 32]));
        assert!(v["items"][0]["original"].is_null());
        assert_eq!(v["items"][0]["thumb"]["cid"], hex::encode([1u8; 32]));
    }
}
