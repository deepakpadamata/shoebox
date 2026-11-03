//! W1-04: the cloud service. Requests go through the router in memory (no sockets), signed the way a device signs.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use shoebox_cloud::{router, Config, Service};
use shoebox_core::cloud::{encrypt_blob, CloudSecret};
use shoebox_core::cloud_auth::{CertBundle, CloudAuth};
use shoebox_core::identity::{issue_device_cert, KeyPair};
use shoebox_core::model::{DeviceCert, Role};
use shoebox_core::share::{self, Manifest, ShareKey, ShareUpload};
use shoebox_core::store::LibraryId;
use shoebox_core::Hlc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tower::ServiceExt;

const LIB: LibraryId = LibraryId([0x5B; 16]);
const T0: i64 = 1_800_000_000_000;

struct Rig {
    svc: Arc<Service>,
    app: axum::Router,
    clock: Arc<AtomicI64>,
    root: KeyPair,
    dir: tempfile::TempDir,
}

fn hlc(ms: u64) -> Hlc {
    Hlc {
        physical_ms: ms,
        counter: 0,
        device: shoebox_core::EndpointId([0; 32]),
    }
}

fn rig_with(edit: impl FnOnce(&mut Config)) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(AtomicI64::new(T0));
    let mut config = Config::new(dir.path().to_path_buf());
    let c = clock.clone();
    config.clock = Arc::new(move || c.load(Ordering::SeqCst));
    edit(&mut config);
    let svc = Service::open(config).unwrap();
    Rig {
        app: router(svc.clone()),
        svc,
        clock,
        root: KeyPair::from_seed([1; 32]),
        dir,
    }
}

fn rig() -> Rig {
    rig_with(|_| {})
}

struct Device {
    key: KeyPair,
    cert: DeviceCert,
}

impl Rig {
    fn device(&self, seed: u8, name: &str) -> Device {
        let key = KeyPair::from_seed([seed; 32]);
        let cert = issue_device_cert(
            &self.root,
            LIB,
            key.endpoint_id(),
            Role::Keeper,
            name,
            hlc(1),
            false,
        );
        Device { key, cert }
    }

    fn now(&self) -> i64 {
        self.clock.load(Ordering::SeqCst)
    }

    fn signed(
        &self,
        d: &Device,
        method: &str,
        path: &str,
        body_hash: [u8; 32],
        time: i64,
    ) -> CloudAuth {
        CloudAuth::sign_hashed(
            &d.key,
            LIB,
            self.root.root_public(),
            None,
            d.cert.clone(),
            method,
            path,
            time,
            &body_hash,
        )
    }

    async fn send(&self, req: Request<Body>) -> (StatusCode, Vec<u8>, axum::http::HeaderMap) {
        let res = self.app.clone().oneshot(req).await.unwrap();
        let (parts, body) = res.into_parts();
        (
            parts.status,
            body.collect().await.unwrap().to_bytes().to_vec(),
            parts.headers,
        )
    }

    /// A signed owner request. `hash_of` is what the signature commits to (the body's hash unless a test lies).
    async fn owner(
        &self,
        d: &Device,
        method: &str,
        path: &str,
        body: Vec<u8>,
    ) -> (StatusCode, Vec<u8>) {
        let hash = *blake3::hash(&body).as_bytes();
        self.owner_hashed(d, method, path, body, hash, self.now())
            .await
    }

    async fn owner_hashed(
        &self,
        d: &Device,
        method: &str,
        path: &str,
        body: Vec<u8>,
        hash_of: [u8; 32],
        time: i64,
    ) -> (StatusCode, Vec<u8>) {
        let auth = self.signed(d, method, path, hash_of, time);
        let req = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("Shoebox {}", auth.to_header()))
            .header("content-length", body.len())
            .body(Body::from(body))
            .unwrap();
        let (s, b, _) = self.send(req).await;
        (s, b)
    }

    async fn put_blob(&self, d: &Device, ciphertext: &[u8]) -> (StatusCode, String) {
        let cid = *blake3::hash(ciphertext).as_bytes();
        let path = format!("/v1/blobs/{}", hex::encode(cid));
        let (s, _) = self
            .owner_hashed(d, "PUT", &path, ciphertext.to_vec(), cid, self.now())
            .await;
        (s, hex::encode(cid))
    }

    async fn public(
        &self,
        path: &str,
        range: Option<&str>,
    ) -> (StatusCode, Vec<u8>, axum::http::HeaderMap) {
        let mut b = Request::builder().method("GET").uri(path);
        if let Some(r) = range {
            b = b.header("range", r);
        }
        self.send(b.body(Body::empty()).unwrap()).await
    }
}

fn share_upload(
    share_id: [u8; 16],
    version: u32,
    cids: Vec<[u8; 32]>,
    expires: Option<i64>,
) -> Vec<u8> {
    let manifest = Manifest {
        album_name: "Trip".into(),
        items: vec![],
        expires: None,
        inbox_key: None,
    };
    let sealed = share::seal(
        &manifest,
        share_id,
        version,
        &ShareKey([7; 32]),
        &KeyPair::from_seed([2; 32]),
    );
    postcard::to_stdvec(&ShareUpload {
        manifest: sealed,
        cids,
        expires_ms: expires,
    })
    .unwrap()
}

fn sealed_photo(tag: u8, len: usize) -> Vec<u8> {
    let data: Vec<u8> = (0..len).map(|i| (i as u8).wrapping_mul(tag)).collect();
    encrypt_blob(&CloudSecret::from_bytes([tag; 32]), &data).ciphertext
}

#[tokio::test]
async fn a_blob_whose_bytes_do_not_hash_to_its_id_is_refused() {
    let r = rig();
    let d = r.device(2, "nest");
    let good = sealed_photo(1, 1000);
    let cid = *blake3::hash(&good).as_bytes();
    let path = format!("/v1/blobs/{}", hex::encode(cid));
    // The signature commits to the id; the body is other bytes.
    let mut bad = good.clone();
    bad[10] ^= 1;
    let (s, body) = r.owner_hashed(&d, "PUT", &path, bad, cid, r.now()).await;
    assert_eq!(
        s,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        String::from_utf8_lossy(&body)
    );
    // Nothing was kept, and the quota was given back.
    assert_eq!(r.svc.store.usage(LIB), 0);
    assert!(r.svc.store.has_blob(LIB, &cid).is_none());
    // The right bytes go in, and twice is fine.
    assert_eq!(r.put_blob(&d, &good).await.0, StatusCode::CREATED);
    assert_eq!(r.put_blob(&d, &good).await.0, StatusCode::OK);
    assert_eq!(r.svc.store.usage(LIB), good.len() as u64);
}

#[tokio::test]
async fn a_share_serves_only_the_blobs_it_lists_and_only_its_own_librarys() {
    let r = rig();
    let d = r.device(2, "nest");
    let (a, b) = (sealed_photo(1, 500), sealed_photo(2, 700));
    let (_, cid_a) = r.put_blob(&d, &a).await;
    let (_, cid_b) = r.put_blob(&d, &b).await;
    let share = [9u8; 16];
    let body = share_upload(share, 1, vec![hex_cid(&cid_a)], None);
    let (s, msg) = r
        .owner(
            &d,
            "PUT",
            &format!("/v1/shares/{}", hex::encode(share)),
            body,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{}", String::from_utf8_lossy(&msg));

    let id = hex::encode(share);
    let (s, got, _) = r.public(&format!("/s/{id}/b/{cid_a}"), None).await;
    assert_eq!((s, got), (StatusCode::OK, a.clone()));
    // b exists in the same library but is not listed: 404, the same as a blob that does not exist.
    let (s, _, _) = r.public(&format!("/s/{id}/b/{cid_b}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _, _) = r
        .public(&format!("/s/{id}/b/{}", hex::encode([0u8; 32])), None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // And another share cannot reach it either, even listing the same id.
    let other = [8u8; 16];
    let body = share_upload(other, 1, vec![hex_cid(&cid_b)], None);
    r.owner(
        &d,
        "PUT",
        &format!("/v1/shares/{}", hex::encode(other)),
        body,
    )
    .await;
    let (s, _, _) = r
        .public(&format!("/s/{}/b/{cid_a}", hex::encode(other)), None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // The manifest comes back whole.
    let (s, m, h) = r.public(&format!("/s/{id}/m"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(share::read_header(&m).unwrap().share_id, share);
    assert_eq!(h["referrer-policy"], "no-referrer");
    assert_eq!(h["access-control-allow-origin"], "*");
}

fn hex_cid(h: &str) -> [u8; 32] {
    hex::decode(h).unwrap().try_into().unwrap()
}

#[tokio::test]
async fn a_deleted_share_and_an_expired_share_stop_answering() {
    let r = rig();
    let d = r.device(2, "nest");
    let (_, cid) = r.put_blob(&d, &sealed_photo(1, 300)).await;
    let (s1, s2) = ([1u8; 16], [2u8; 16]);
    for (id, exp) in [(s1, None), (s2, Some(T0 + 60_000))] {
        let body = share_upload(id, 1, vec![hex_cid(&cid)], exp);
        assert_eq!(
            r.owner(&d, "PUT", &format!("/v1/shares/{}", hex::encode(id)), body)
                .await
                .0,
            StatusCode::OK
        );
    }
    let m1 = format!("/s/{}/m", hex::encode(s1));
    let m2 = format!("/s/{}/m", hex::encode(s2));
    assert_eq!(r.public(&m1, None).await.0, StatusCode::OK);
    assert_eq!(r.public(&m2, None).await.0, StatusCode::OK);
    // Revoke the first: gone at once.
    let (s, _) = r
        .owner(
            &d,
            "DELETE",
            &format!("/v1/shares/{}", hex::encode(s1)),
            vec![],
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(r.public(&m1, None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(
        r.public(&format!("/s/{}/b/{cid}", hex::encode(s1)), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    // The second expires with the clock, for the manifest and for blobs.
    r.clock.store(T0 + 60_000, Ordering::SeqCst);
    assert_eq!(r.public(&m2, None).await.0, StatusCode::GONE);
    assert_eq!(
        r.public(&format!("/s/{}/b/{cid}", hex::encode(s2)), None)
            .await
            .0,
        StatusCode::GONE
    );
    // After the grace period the janitor deletes it.
    assert_eq!(r.svc.store.sweep_expired(T0 + 60_000), 1);
    assert_eq!(r.public(&m2, None).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_revoked_device_an_unknown_library_and_a_stale_request_are_refused() {
    let r = rig();
    let a = r.device(2, "nest");
    let b = r.device(3, "mac");
    assert_eq!(
        r.owner(&a, "GET", "/v1/usage", vec![]).await.0,
        StatusCode::OK
    );
    assert_eq!(
        r.owner(&b, "GET", "/v1/usage", vec![]).await.0,
        StatusCode::OK
    );

    // The service learns that b was revoked (a member sends the revocation).
    let revoked = issue_device_cert(
        &r.root,
        LIB,
        b.key.endpoint_id(),
        Role::Keeper,
        "mac",
        hlc(5),
        true,
    );
    let bundle = postcard::to_stdvec(&CertBundle {
        admins: vec![],
        devices: vec![revoked],
    })
    .unwrap();
    assert_eq!(
        r.owner(&a, "POST", "/v1/library/certs", bundle).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        r.owner(&b, "GET", "/v1/usage", vec![]).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        r.owner(&a, "GET", "/v1/usage", vec![]).await.0,
        StatusCode::OK
    );

    // A device certified by someone who is not the library's root is not a member.
    let stranger = KeyPair::from_seed([77; 32]);
    let forged = issue_device_cert(
        &stranger,
        LIB,
        stranger.endpoint_id(),
        Role::Keeper,
        "x",
        hlc(1),
        false,
    );
    let intruder = Device {
        key: stranger,
        cert: forged,
    };
    assert_eq!(
        r.owner(&intruder, "GET", "/v1/usage", vec![]).await.0,
        StatusCode::FORBIDDEN
    );
    // The library's root key is fixed by its first request: another root cannot take it over.
    let other_root = rig();
    let hijack_root = KeyPair::from_seed([55; 32]);
    let key = KeyPair::from_seed([56; 32]);
    let cert = issue_device_cert(
        &hijack_root,
        LIB,
        key.endpoint_id(),
        Role::Keeper,
        "h",
        hlc(1),
        false,
    );
    let auth = CloudAuth::sign(
        &key,
        LIB,
        hijack_root.root_public(),
        None,
        cert,
        "GET",
        "/v1/usage",
        r.now(),
        b"",
    );
    let req = Request::builder()
        .uri("/v1/usage")
        .header("authorization", format!("Shoebox {}", auth.to_header()))
        .body(Body::empty())
        .unwrap();
    assert_eq!(r.send(req).await.0, StatusCode::FORBIDDEN);
    drop(other_root);

    // Unsigned, signed too long ago, and signed for another path.
    let (s, _, _) = r.public("/v1/usage", None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let empty = *blake3::hash(b"").as_bytes();
    assert_eq!(
        r.owner_hashed(&a, "GET", "/v1/usage", vec![], empty, r.now() - 10 * 60_000)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let auth = r.signed(&a, "GET", "/v1/shares", empty, r.now());
    let req = Request::builder()
        .uri("/v1/usage")
        .header("authorization", format!("Shoebox {}", auth.to_header()))
        .body(Body::empty())
        .unwrap();
    assert_eq!(r.send(req).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_closed_service_refuses_libraries_it_has_not_registered() {
    let r = rig_with(|c| c.open_registration = false);
    let d = r.device(2, "nest");
    assert_eq!(
        r.owner(&d, "GET", "/v1/usage", vec![]).await.0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn range_requests_quotas_and_daily_limits() {
    let r = rig_with(|c| {
        c.library_quota = 5_000;
        c.share_daily_bytes = 1_500;
    });
    let d = r.device(2, "nest");
    let video = sealed_photo(3, 1000); // 1016 bytes
    let (_, cid) = r.put_blob(&d, &video).await;
    let share = [4u8; 16];
    let id = hex::encode(share);
    let body = share_upload(share, 1, vec![hex_cid(&cid)], None);
    assert_eq!(
        r.owner(&d, "PUT", &format!("/v1/shares/{id}"), body)
            .await
            .0,
        StatusCode::OK
    );

    let path = format!("/s/{id}/b/{cid}");
    let (s, got, h) = r.public(&path, Some("bytes=100-199")).await;
    assert_eq!(s, StatusCode::PARTIAL_CONTENT);
    assert_eq!(got, &video[100..200]);
    assert_eq!(
        h["content-range"],
        format!("bytes 100-199/{}", video.len()).as_str()
    );
    let (s, got, _) = r.public(&path, Some("bytes=-16")).await;
    assert_eq!(
        (s, got),
        (
            StatusCode::PARTIAL_CONTENT,
            video[video.len() - 16..].to_vec()
        )
    );
    let (s, _, _) = r.public(&path, Some("bytes=5000-")).await;
    assert_eq!(s, StatusCode::RANGE_NOT_SATISFIABLE);
    // 116 bytes served so far today; a whole 1016 more passes 1,500 only once.
    let (s, _, _) = r.public(&path, None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, body, _) = r.public(&path, None).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    assert!(String::from_utf8_lossy(&body).contains("daily-limit"));
    // The next day the link serves again.
    r.clock.store(T0 + 86_400_000, Ordering::SeqCst);
    assert_eq!(r.public(&path, None).await.0, StatusCode::OK);

    // Quota: 5,000 bytes in all; the blob and manifest are in, a 4,000 byte blob is not.
    let big = sealed_photo(4, 4000);
    let (s, _) = r.put_blob(&d, &big).await;
    assert_eq!(s, StatusCode::INSUFFICIENT_STORAGE);
    let (_, body) = r.owner(&d, "GET", "/v1/usage", vec![]).await;
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["quota"], 5000);
    assert!(v["used"].as_u64().unwrap() < 5000);
}

#[tokio::test]
async fn a_share_cannot_list_blobs_that_were_not_uploaded_or_roll_back() {
    let r = rig();
    let d = r.device(2, "nest");
    let (_, cid) = r.put_blob(&d, &sealed_photo(1, 100)).await;
    let share = [6u8; 16];
    let path = format!("/v1/shares/{}", hex::encode(share));
    let ghost = [0xEE; 32];
    let (s, body) = r
        .owner(
            &d,
            "PUT",
            &path,
            share_upload(share, 1, vec![hex_cid(&cid), ghost], None),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(String::from_utf8_lossy(&body).contains(&hex::encode(ghost)));
    assert_eq!(
        r.owner(
            &d,
            "PUT",
            &path,
            share_upload(share, 2, vec![hex_cid(&cid)], None)
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        r.owner(
            &d,
            "PUT",
            &path,
            share_upload(share, 1, vec![hex_cid(&cid)], None)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    // The manifest must be for the id in the path.
    let wrong = share_upload([7; 16], 3, vec![hex_cid(&cid)], None);
    assert_eq!(
        r.owner(&d, "PUT", &path, wrong).await.0,
        StatusCode::BAD_REQUEST
    );
    // A blob a share lists cannot be deleted until the share is.
    let (s, body) = r
        .owner(&d, "DELETE", &format!("/v1/blobs/{cid}"), vec![])
        .await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "{}",
        String::from_utf8_lossy(&body)
    );
    r.owner(&d, "DELETE", &path, vec![]).await;
    assert_eq!(
        r.owner(&d, "DELETE", &format!("/v1/blobs/{cid}"), vec![])
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(r.svc.store.usage(LIB), 0);
}

#[tokio::test]
async fn blobs_are_listed_probed_read_back_and_objects_hold_library_state() {
    let r = rig();
    let d = r.device(2, "nest");
    let mut cids = vec![];
    for t in 1..=5u8 {
        cids.push(r.put_blob(&d, &sealed_photo(t, 200 + t as usize)).await.1);
    }
    cids.sort();
    let (_, body) = r.owner(&d, "GET", "/v1/blobs?limit=2", vec![]).await;
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["blobs"].as_array().unwrap().len(), 2);
    let next = v["next"].as_str().unwrap().to_string();
    assert_eq!(next, cids[1]);
    let (_, body) = r
        .owner(
            &d,
            "GET",
            &format!("/v1/blobs?after={next}&limit=10"),
            vec![],
        )
        .await;
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["blobs"].as_array().unwrap().len(), 3);
    assert!(v["next"].is_null());

    let asked = serde_json::to_vec(&[cids[0].clone(), hex::encode([1u8; 32])]).unwrap();
    let (_, body) = r.owner(&d, "POST", "/v1/blobs/has", asked).await;
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["present"], serde_json::json!([cids[0]]));

    let (s, got) = r
        .owner(&d, "GET", &format!("/v1/blobs/{}", cids[2]), vec![])
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(hex::encode(blake3::hash(&got).as_bytes()), cids[2]);

    // An object (an encrypted library snapshot), replaced by a newer one.
    let snap1 = vec![1u8; 3000];
    let snap2 = vec![2u8; 2000];
    for snap in [&snap1, &snap2] {
        let hash = *blake3::hash(snap).as_bytes();
        let auth = r.signed(&d, "PUT", "/v1/objects/library-snapshot", hash, r.now());
        let req = Request::builder()
            .method("PUT")
            .uri("/v1/objects/library-snapshot")
            .header("authorization", format!("Shoebox {}", auth.to_header()))
            .header("x-content-blake3", hex::encode(hash))
            .header("content-length", snap.len())
            .body(Body::from(snap.clone()))
            .unwrap();
        assert_eq!(r.send(req).await.0, StatusCode::CREATED);
    }
    let before = r.svc.store.usage(LIB);
    let (s, got) = r
        .owner(&d, "GET", "/v1/objects/library-snapshot", vec![])
        .await;
    assert_eq!((s, got), (StatusCode::OK, snap2.clone()));
    assert!(before >= 2000);
    assert_eq!(
        r.owner(&d, "GET", "/v1/objects/.hidden", vec![]).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        r.owner(&d, "DELETE", "/v1/objects/library-snapshot", vec![])
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(r.svc.store.usage(LIB), before - 2000);
}

#[tokio::test]
async fn state_survives_a_restart() {
    let r = rig();
    let d = r.device(2, "nest");
    let (_, cid) = r.put_blob(&d, &sealed_photo(1, 400)).await;
    let share = [3u8; 16];
    let id = hex::encode(share);
    let body = share_upload(share, 1, vec![hex_cid(&cid)], None);
    r.owner(&d, "PUT", &format!("/v1/shares/{id}"), body).await;
    let used = r.svc.store.usage(LIB);

    let mut config = Config::new(r.dir.path().to_path_buf());
    let c = r.clock.clone();
    config.clock = Arc::new(move || c.load(Ordering::SeqCst));
    let svc = Service::open(config).unwrap();
    assert_eq!(svc.store.usage(LIB), used);
    let app = router(svc);
    let res = app
        .oneshot(
            Request::builder()
                .uri(format!("/s/{id}/b/{cid}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

/// The service holds no key and no plaintext: after a share is made, nothing in its data directory contains
/// the photo, the blob keys, the share key or the cloud secret.
#[tokio::test]
async fn the_data_directory_holds_no_plaintext_or_key() {
    let r = rig();
    let d = r.device(2, "nest");
    let secret = CloudSecret::from_bytes([0x42; 32]);
    let photo: Vec<u8> = b"THE-FAMILY-PHOTO-BYTES-"
        .iter()
        .cycle()
        .take(50_000)
        .copied()
        .collect();
    let sealed = encrypt_blob(&secret, &photo);
    let (_, cid) = r.put_blob(&d, &sealed.ciphertext).await;
    let share = [5u8; 16];
    let body = share_upload(share, 1, vec![hex_cid(&cid)], None);
    r.owner(
        &d,
        "PUT",
        &format!("/v1/shares/{}", hex::encode(share)),
        body,
    )
    .await;

    let needles: Vec<(&str, Vec<u8>)> = vec![
        ("photo bytes", b"THE-FAMILY-PHOTO-BYTES-".to_vec()),
        ("blob key", sealed.key.0.to_vec()),
        ("cloud secret", secret.as_bytes().to_vec()),
        ("share key", vec![7u8; 32]),
        ("album name", b"Trip".to_vec()),
    ];
    let mut seen = 0;
    let mut stack = vec![r.dir.path().to_path_buf()];
    while let Some(p) = stack.pop() {
        for e in std::fs::read_dir(&p).unwrap() {
            let path = e.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let bytes = std::fs::read(&path).unwrap();
            seen += 1;
            for (what, needle) in &needles {
                assert!(
                    !bytes.windows(needle.len()).any(|w| w == needle.as_slice()),
                    "{what} found in {}",
                    path.display()
                );
            }
        }
    }
    assert!(seen >= 3, "looked at {seen} files");
}

#[tokio::test]
async fn the_viewer_page_is_served_with_a_strict_policy() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "<!doctype html>viewer").unwrap();
    std::fs::write(dir.path().join("app.wasm"), [0, 97, 115, 109]).unwrap();
    let viewer = dir.path().to_path_buf();
    let r = rig_with(|c| {
        c.viewer_dir = Some(viewer);
        c.csp_connect = vec!["https://relay.example".into()];
    });
    let (s, body, h) = r.public("/", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body, b"<!doctype html>viewer");
    let csp = h["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("default-src 'none'") && csp.contains("frame-ancestors 'none'"));
    assert!(csp.contains("connect-src 'self' https://relay.example"));
    assert_eq!(h["referrer-policy"], "no-referrer");
    assert_eq!(r.public("/a", None).await.0, StatusCode::OK);
    let (s, _, h) = r.public("/app.wasm", None).await;
    assert_eq!(
        (s, h["content-type"].to_str().unwrap()),
        (StatusCode::OK, "application/wasm")
    );
    // No way out of the directory.
    assert_eq!(
        r.public("/../Cargo.toml", None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        r.public("/%2e%2e/secret", None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(r.public("/missing.js", None).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_reported_share_is_switched_off_at_once_and_the_owner_sees_why() {
    let r = rig();
    let d = r.device(2, "nest");
    let (_, cid) = r.put_blob(&d, &sealed_photo(1, 500)).await;
    let share = [9u8; 16];
    let id = hex::encode(share);
    let body = share_upload(share, 1, vec![hex_cid(&cid)], None);
    r.owner(&d, "PUT", &format!("/v1/shares/{id}"), body.clone())
        .await;
    assert_eq!(
        r.public(&format!("/s/{id}/m"), None).await.0,
        StatusCode::OK
    );

    // Anyone with the link reports it, with a few words.
    let report = |reason: &'static str| {
        let req = Request::builder()
            .method("POST")
            .uri(format!("/report/{id}"))
            .body(Body::from(reason))
            .unwrap();
        r.send(req)
    };
    assert_eq!(report("not mine\u{7}").await.0, StatusCode::ACCEPTED);
    for path in [format!("/s/{id}/m"), format!("/s/{id}/b/{cid}")] {
        let (s, body, _) = r.public(&path, None).await;
        assert_eq!(s, StatusCode::GONE);
        assert!(String::from_utf8_lossy(&body).contains("reported"));
    }

    // The owner's list says so, with the reason (control characters removed), and refreshing the share does not
    // switch it back on.
    let (s, list) = r.owner(&d, "GET", "/v1/shares", vec![]).await;
    assert_eq!(s, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(&list).unwrap();
    assert_eq!(v["shares"][0]["reported"]["reason"], "not mine");
    r.owner(&d, "PUT", &format!("/v1/shares/{id}"), body).await;
    assert_eq!(
        r.public(&format!("/s/{id}/m"), None).await.0,
        StatusCode::GONE
    );

    // Reinstating is the operator deleting the report; a share that does not exist cannot be reported.
    std::fs::remove_file(r.dir.path().join("reports").join(&id)).unwrap();
    assert_eq!(
        r.public(&format!("/s/{id}/m"), None).await.0,
        StatusCode::OK
    );
    let nowhere = Request::builder()
        .method("POST")
        .uri(format!("/report/{}", hex::encode([1u8; 16])))
        .body(Body::empty())
        .unwrap();
    assert_eq!(r.send(nowhere).await.0, StatusCode::NOT_FOUND);
}
