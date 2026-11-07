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

mod common;
use common::FakeS3;

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
    rig_on(Arc::new(AtomicI64::new(T0)), edit)
}

fn rig_on(clock: Arc<AtomicI64>, edit: impl FnOnce(&mut Config)) -> Rig {
    let dir = tempfile::tempdir().unwrap();
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

/// A service that keeps the bytes in a (fake) bucket and only the bookkeeping on disk; one clock for both.
async fn rig_in_bucket(edit: impl FnOnce(&mut Config)) -> (Rig, FakeS3) {
    let clock = Arc::new(AtomicI64::new(T0));
    let c = clock.clone();
    let fake = common::start(Arc::new(move || c.load(Ordering::SeqCst))).await;
    let s3 = fake.config.clone();
    let r = rig_on(clock, |c| {
        c.s3 = Some(s3);
        edit(c);
    });
    (r, fake)
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

    /// `POST /v1/uploads` for blobs `(cid, size)` and objects `(name, size, hash)`.
    async fn begin(
        &self,
        d: &Device,
        blobs: &[(String, u64)],
        objects: &[(&str, u64, [u8; 32])],
    ) -> (StatusCode, serde_json::Value) {
        let body = serde_json::json!({
            "blobs": blobs.iter().map(|(c, n)| serde_json::json!({ "cid": c, "size": n })).collect::<Vec<_>>(),
            "objects": objects.iter().map(|(name, n, h)| serde_json::json!({ "name": name, "size": n, "hash": hex::encode(h) })).collect::<Vec<_>>(),
        });
        let (s, b) = self
            .owner(d, "POST", "/v1/uploads", serde_json::to_vec(&body).unwrap())
            .await;
        (s, serde_json::from_slice(&b).unwrap_or_default())
    }

    /// Put bytes where an upload slot says: the service itself (a path) or the bucket (a URL).
    async fn put_bytes(&self, url: &str, bytes: Vec<u8>) -> (StatusCode, Vec<u8>) {
        if url.starts_with('/') {
            let req = Request::builder()
                .method("PUT")
                .uri(url)
                .header("content-length", bytes.len())
                .body(Body::from(bytes))
                .unwrap();
            let (s, b, _) = self.send(req).await;
            return (s, b);
        }
        let res = common::http()
            .put(url)
            .header("content-length", bytes.len())
            .body(bytes)
            .send()
            .await
            .unwrap();
        let s = StatusCode::from_u16(res.status().as_u16()).unwrap();
        (s, res.bytes().await.unwrap().to_vec())
    }

    async fn commit(&self, d: &Device, tokens: &[&str]) -> serde_json::Value {
        let body = serde_json::to_vec(&serde_json::json!({ "tokens": tokens })).unwrap();
        let (s, b) = self.owner(d, "POST", "/v1/uploads/commit", body).await;
        assert_eq!(s, StatusCode::OK, "{}", String::from_utf8_lossy(&b));
        serde_json::from_slice(&b).unwrap()
    }

    /// Upload one blob the whole way: CREATED when it went up, OK when the service had it, else the refusal.
    async fn put_blob(&self, d: &Device, ciphertext: &[u8]) -> (StatusCode, String) {
        let cid = hex::encode(blake3::hash(ciphertext).as_bytes());
        let (s, v) = self
            .begin(d, &[(cid.clone(), ciphertext.len() as u64)], &[])
            .await;
        if s != StatusCode::OK {
            return (s, cid);
        }
        let slot = &v["uploads"][0];
        if slot["stored"] == true {
            return (StatusCode::OK, cid);
        }
        let (s, _) = self
            .put_bytes(slot["url"].as_str().unwrap(), ciphertext.to_vec())
            .await;
        if !s.is_success() {
            return (s, cid);
        }
        let done = self.commit(d, &[slot["token"].as_str().unwrap()]).await;
        if done["results"][0]["ok"] != true {
            return (StatusCode::CONFLICT, cid);
        }
        (StatusCode::CREATED, cid)
    }

    async fn put_object(&self, d: &Device, name: &str, bytes: &[u8]) -> StatusCode {
        let hash = *blake3::hash(bytes).as_bytes();
        let (s, v) = self
            .begin(d, &[], &[(name, bytes.len() as u64, hash)])
            .await;
        if s != StatusCode::OK {
            return s;
        }
        let slot = &v["uploads"][0];
        let (s, _) = self
            .put_bytes(slot["url"].as_str().unwrap(), bytes.to_vec())
            .await;
        if !s.is_success() {
            return s;
        }
        let done = self.commit(d, &[slot["token"].as_str().unwrap()]).await;
        if done["results"][0]["ok"] != true {
            return StatusCode::CONFLICT;
        }
        StatusCode::CREATED
    }

    /// A GET that follows a redirect to the bucket, as a browser does.
    async fn fetch(&self, path: &str) -> (StatusCode, Vec<u8>) {
        let (s, b, h) = self.public(path, None).await;
        if s != StatusCode::FOUND {
            return (s, b);
        }
        let res = common::http()
            .get(h["location"].to_str().unwrap())
            .send()
            .await
            .unwrap();
        let s = StatusCode::from_u16(res.status().as_u16()).unwrap();
        (s, res.bytes().await.unwrap().to_vec())
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
    // The upload is asked for under the id; the bytes sent are other bytes.
    let (_, v) = r
        .begin(&d, &[(hex::encode(cid), good.len() as u64)], &[])
        .await;
    let url = v["uploads"][0]["url"].as_str().unwrap().to_string();
    let mut bad = good.clone();
    bad[10] ^= 1;
    let (s, body) = r.put_bytes(&url, bad).await;
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
    assert_eq!(r.svc.store.sweep_expired(T0 + 60_000).len(), 1);
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
    // The operator's own library is let in while closed.
    let r = rig_with(|c| {
        c.open_registration = false;
        c.allowed_libraries = vec![LIB.0];
    });
    let d = r.device(2, "nest");
    assert_eq!(
        r.owner(&d, "GET", "/v1/usage", vec![]).await.0,
        StatusCode::OK
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
        assert_eq!(
            r.put_object(&d, "library-snapshot", snap).await,
            StatusCode::CREATED
        );
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

    // A share that does not exist cannot be reported.
    let nowhere = Request::builder()
        .method("POST")
        .uri(format!("/report/{}", hex::encode([1u8; 16])))
        .body(Body::empty())
        .unwrap();
    assert_eq!(r.send(nowhere).await.0, StatusCode::NOT_FOUND);
}

impl Rig {
    async fn admin(&self, method: &str, path: &str, token: Option<&str>) -> (StatusCode, Vec<u8>) {
        let mut b = Request::builder().method(method).uri(path);
        if let Some(t) = token {
            b = b.header("authorization", format!("Bearer {t}"));
        }
        let (s, body, _) = self.send(b.body(Body::empty()).unwrap()).await;
        (s, body)
    }
}

#[tokio::test]
async fn the_operator_lists_reports_and_reinstates_or_deletes_with_the_admin_token() {
    // Without a token there is no operator interface at all.
    let closed = rig();
    assert_eq!(
        closed.admin("GET", "/admin/reports", Some("")).await.0,
        StatusCode::NOT_FOUND
    );

    let r = rig_with(|c| c.admin_token = Some("operator-secret".into()));
    let d = r.device(2, "nest");
    let (_, cid) = r.put_blob(&d, &sealed_photo(1, 500)).await;
    let mut ids = vec![];
    for n in [7u8, 8] {
        let share = [n; 16];
        let id = hex::encode(share);
        r.owner(
            &d,
            "PUT",
            &format!("/v1/shares/{id}"),
            share_upload(share, 1, vec![hex_cid(&cid)], None),
        )
        .await;
        let req = Request::builder()
            .method("POST")
            .uri(format!("/report/{id}"))
            .body(Body::from(format!("report {n}")))
            .unwrap();
        assert_eq!(r.send(req).await.0, StatusCode::ACCEPTED);
        ids.push(id);
    }

    for token in [None, Some("wrong"), Some("operator-secret ")] {
        assert_eq!(
            r.admin("GET", "/admin/reports", token).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            r.admin(
                "POST",
                &format!("/admin/shares/{}/reinstate", ids[0]),
                token
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    let (s, body) = r
        .admin("GET", "/admin/reports", Some("operator-secret"))
        .await;
    assert_eq!(s, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let reports = v["reports"].as_array().unwrap();
    assert_eq!(reports.len(), 2);
    assert!(reports.iter().any(|x| x["shareId"] == ids[0].as_str()
        && x["reason"] == "report 7"
        && x["library"] == hex::encode(LIB.0).as_str()));

    // Reinstated: served again, off the list. Deleted: gone for everyone, owner included.
    let reinstate = format!("/admin/shares/{}/reinstate", ids[0]);
    assert_eq!(
        r.admin("POST", &reinstate, Some("operator-secret")).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        r.public(&format!("/s/{}/m", ids[0]), None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        r.admin("POST", &reinstate, Some("operator-secret")).await.0,
        StatusCode::NOT_FOUND,
        "nothing left to reinstate"
    );
    assert_eq!(
        r.admin(
            "DELETE",
            &format!("/admin/shares/{}", ids[1]),
            Some("operator-secret")
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        r.public(&format!("/s/{}/m", ids[1]), None).await.0,
        StatusCode::NOT_FOUND
    );
    let (_, list) = r.owner(&d, "GET", "/v1/shares", vec![]).await;
    let v: serde_json::Value = serde_json::from_slice(&list).unwrap();
    assert_eq!(v["shares"].as_array().unwrap().len(), 1);
    let (_, body) = r
        .admin("GET", "/admin/reports", Some("operator-secret"))
        .await;
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(v["reports"].as_array().unwrap().is_empty());
    // The blob stays with the library.
    assert!(r.svc.store.has_blob(LIB, &hex_cid(&cid)).is_some());
}

/// The Cloudflare Durable Object keeps its records in SQLite and may be evicted between any two requests; here
/// the records are in memory and the service is opened again on them, mid-upload.
#[tokio::test]
async fn with_records_kept_elsewhere_an_upload_and_a_report_survive_a_reopen() {
    let clock = Arc::new(AtomicI64::new(T0));
    let c = clock.clone();
    let bucket = common::start(Arc::new(move || c.load(Ordering::SeqCst))).await;
    let meta: Arc<dyn shoebox_cloud::meta::Meta> =
        Arc::new(shoebox_cloud::meta::MemMeta::default());
    let dir = tempfile::tempdir().unwrap();
    let open = || {
        let mut config = Config::new(dir.path().join("unused"));
        config.s3 = Some(bucket.config.clone());
        let c = clock.clone();
        config.clock = Arc::new(move || c.load(Ordering::SeqCst));
        let svc = Service::open_with(
            config,
            meta.clone(),
            None,
            Arc::new(shoebox_cloud::s3::Reqwest::new().unwrap()),
        )
        .unwrap();
        Rig {
            app: router(svc.clone()),
            svc,
            clock: clock.clone(),
            root: KeyPair::from_seed([1; 32]),
            dir: tempfile::tempdir().unwrap(),
        }
    };

    let r = open();
    let d = r.device(2, "nest");
    let (_, kept) = r.put_blob(&d, &sealed_photo(1, 400)).await;
    let share = [5u8; 16];
    let id = hex::encode(share);
    r.owner(
        &d,
        "PUT",
        &format!("/v1/shares/{id}"),
        share_upload(share, 1, vec![hex_cid(&kept)], None),
    )
    .await;
    let req = Request::builder()
        .method("POST")
        .uri(format!("/report/{id}"))
        .body(Body::from("spam"))
        .unwrap();
    assert_eq!(r.send(req).await.0, StatusCode::ACCEPTED);
    let photo = sealed_photo(2, 900);
    let cid = hex::encode(blake3::hash(&photo).as_bytes());
    let (_, v) = r.begin(&d, &[(cid.clone(), photo.len() as u64)], &[]).await;
    let url = v["uploads"][0]["url"].as_str().unwrap().to_string();
    let token = v["uploads"][0]["token"].as_str().unwrap().to_string();
    let used = r.svc.store.usage(LIB);
    assert!(used >= 400 + 900, "the blob, the share and the reservation");
    drop(r);

    // Opened again: the reservation still counts, the upload commits, the report still stands.
    let r = open();
    assert_eq!(r.svc.store.usage(LIB), used);
    assert_eq!(r.put_bytes(&url, photo).await.0, StatusCode::OK);
    assert_eq!(r.commit(&d, &[&token]).await["results"][0]["ok"], true);
    assert_eq!(r.svc.store.usage(LIB), used);
    assert!(r.svc.store.has_blob(LIB, &hex_cid(&cid)).is_some());
    assert_eq!(
        r.public(&format!("/s/{id}/m"), None).await.0,
        StatusCode::GONE
    );
    assert!(meta.children("uploads").unwrap().is_empty());
    drop(r);

    // An upload begun and never made is cleaned up by the janitor of a later instance.
    let r = open();
    assert!(r.svc.store.has_blob(LIB, &hex_cid(&cid)).is_some());
    let (_, v) = r.begin(&d, &[(hex::encode([3u8; 32]), 300)], &[]).await;
    assert!(v["uploads"][0]["token"].is_string());
    drop(r);
    let r = open();
    assert_eq!(r.svc.store.usage(LIB), used + 300);
    r.clock.fetch_add(2 * 3600 * 1000 + 1, Ordering::SeqCst);
    r.svc.sweep().await;
    assert_eq!(r.svc.store.usage(LIB), used);
    assert!(meta.children("uploads").unwrap().is_empty());
    assert!(bucket.refused().is_empty(), "{:?}", bucket.refused());
}

// ---- with the bytes in a bucket ----

fn files_under(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    let mut stack = vec![dir.to_path_buf()];
    while let Some(p) = stack.pop() {
        for e in std::fs::read_dir(&p).unwrap() {
            let path = e.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out
}

#[tokio::test]
async fn in_a_bucket_uploads_go_straight_there_and_reads_are_redirected() {
    let viewer = tempfile::tempdir().unwrap();
    std::fs::write(viewer.path().join("index.html"), "<!doctype html>").unwrap();
    let v = viewer.path().to_path_buf();
    let (r, bucket) = rig_in_bucket(|c| c.viewer_dir = Some(v)).await;
    let d = r.device(2, "nest");
    let photo = sealed_photo(1, 5000);
    let other = sealed_photo(2, 300);
    let (s, cid) = r.put_blob(&d, &photo).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(
        r.put_blob(&d, &photo).await.0,
        StatusCode::OK,
        "held already"
    );
    let (_, other_cid) = r.put_blob(&d, &other).await;
    let lib = hex::encode(LIB.0);
    assert_eq!(
        bucket.get(&format!("libs/{lib}/blobs/{cid}")).unwrap(),
        photo,
        "the bytes went to the bucket"
    );
    assert_eq!(r.svc.store.usage(LIB), (photo.len() + other.len()) as u64);

    let share = [9u8; 16];
    let id = hex::encode(share);
    let body = share_upload(share, 1, vec![hex_cid(&cid)], None);
    let sealed_manifest = postcard::from_bytes::<ShareUpload>(&body).unwrap().manifest;
    assert_eq!(
        r.owner(&d, "PUT", &format!("/v1/shares/{id}"), body)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        bucket.get(&format!("shares/{id}/manifest")).unwrap(),
        sealed_manifest
    );

    // A reader is sent to the bucket for what the share lists, and only for that.
    let (s, _, h) = r.public(&format!("/s/{id}/b/{cid}"), None).await;
    assert_eq!(s, StatusCode::FOUND);
    let to = h["location"].to_str().unwrap();
    assert!(to.starts_with(bucket.config.endpoint.as_str()), "{to}");
    assert_eq!(h["cache-control"], "private, max-age=300");
    assert_eq!(r.fetch(&format!("/s/{id}/b/{cid}")).await.1, photo);
    assert_eq!(r.fetch(&format!("/s/{id}/m")).await.1, sealed_manifest);
    assert_eq!(
        r.public(&format!("/s/{id}/b/{other_cid}"), None).await.0,
        StatusCode::NOT_FOUND
    );
    // The owner is sent there too (the client drops its signature on the way).
    let (s, _) = r
        .owner(&d, "GET", &format!("/v1/blobs/{other_cid}"), vec![])
        .await;
    assert_eq!(s, StatusCode::FOUND);

    // The page may fetch from the bucket, since that is where reads are sent.
    let (_, _, h) = r.public("/a", None).await;
    let csp = h["content-security-policy"].to_str().unwrap();
    let origin = bucket.config.endpoint.origin().ascii_serialization();
    assert!(
        csp.contains(&format!("connect-src 'self' {origin}")),
        "{csp}"
    );

    // The data directory holds bookkeeping only.
    for f in files_under(r.dir.path()) {
        let bytes = std::fs::read(&f).unwrap();
        assert!(
            !bytes
                .windows(64)
                .any(|w| photo.windows(64).next() == Some(w)),
            "{} holds blob bytes",
            f.display()
        );
        assert!(!f.ends_with("manifest"), "{}", f.display());
    }

    // Stopping the share deletes its manifest; deleting a blob deletes its bytes.
    r.owner(&d, "DELETE", &format!("/v1/shares/{id}"), vec![])
        .await;
    assert!(bucket.get(&format!("shares/{id}/manifest")).is_none());
    assert_eq!(
        r.public(&format!("/s/{id}/b/{cid}"), None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        r.owner(&d, "DELETE", &format!("/v1/blobs/{cid}"), vec![])
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert!(bucket.get(&format!("libs/{lib}/blobs/{cid}")).is_none());
    assert_eq!(r.svc.store.usage(LIB), other.len() as u64);
    assert!(bucket.refused().is_empty(), "{:?}", bucket.refused());
}

#[tokio::test]
async fn a_bucket_upload_of_another_size_late_or_never_made_is_not_recorded() {
    let (r, bucket) = rig_in_bucket(|_| {}).await;
    let d = r.device(2, "nest");
    let photo = sealed_photo(1, 1000);
    let cid = hex::encode(blake3::hash(&photo).as_bytes());
    let (_, v) = r.begin(&d, &[(cid.clone(), photo.len() as u64)], &[]).await;
    let url = v["uploads"][0]["url"].as_str().unwrap().to_string();
    let token = v["uploads"][0]["token"].as_str().unwrap().to_string();
    assert_eq!(r.svc.store.usage(LIB), photo.len() as u64, "reserved");

    // Committed before the bytes are there: refused, still waiting.
    let done = r.commit(&d, &[&token]).await;
    assert_eq!(done["results"][0]["error"], "not-uploaded");
    // The length is signed: the bucket refuses other bytes, and the URL for any other use.
    let (s, _) = r.put_bytes(&url, photo[..999].to_vec()).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let res = common::http().get(&url).send().await.unwrap();
    assert_eq!(res.status().as_u16(), 403);
    assert!(bucket.keys().is_empty());

    // Two hours on, the URL has run out and the janitor gives the quota back.
    r.clock.fetch_add(2 * 3600 * 1000 + 1, Ordering::SeqCst);
    let (s, _) = r.put_bytes(&url, photo.clone()).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    r.svc.sweep().await;
    assert_eq!(r.svc.store.usage(LIB), 0);
    assert_eq!(
        r.commit(&d, &[&token]).await["results"][0]["error"],
        "unknown-upload"
    );
    assert!(r.svc.store.has_blob(LIB, &hex_cid(&cid)).is_none());

    // Bytes that reached the bucket but were never committed are deleted when the upload runs out.
    let (_, v) = r.begin(&d, &[(cid.clone(), photo.len() as u64)], &[]).await;
    let url = v["uploads"][0]["url"].as_str().unwrap().to_string();
    assert_eq!(r.put_bytes(&url, photo.clone()).await.0, StatusCode::OK);
    assert_eq!(bucket.keys().len(), 1);
    r.clock.fetch_add(2 * 3600 * 1000 + 1, Ordering::SeqCst);
    r.svc.sweep().await;
    assert!(bucket.keys().is_empty());
    assert_eq!(r.svc.store.usage(LIB), 0);
    let refused = bucket.refused();
    assert!(
        refused
            .iter()
            .all(|w| w.contains("SignatureDoesNotMatch") || w.contains("expired")),
        "{refused:?}"
    );
}

#[tokio::test]
async fn what_a_bucket_holds_survives_a_restart_and_expired_shares_leave_it() {
    let (r, bucket) = rig_in_bucket(|_| {}).await;
    let d = r.device(2, "nest");
    let (_, keep) = r.put_blob(&d, &sealed_photo(1, 400)).await;
    let (_, gone) = r.put_blob(&d, &sealed_photo(2, 500)).await;
    assert_eq!(
        r.put_object(&d, "state.abc", &[3u8; 700]).await,
        StatusCode::CREATED
    );
    assert_eq!(
        r.put_object(&d, "state.abc", &[4u8; 600]).await,
        StatusCode::CREATED
    );
    r.owner(&d, "DELETE", &format!("/v1/blobs/{gone}"), vec![])
        .await;
    let share = [8u8; 16];
    let id = hex::encode(share);
    let body = share_upload(share, 1, vec![hex_cid(&keep)], Some(T0 + 60_000));
    r.owner(&d, "PUT", &format!("/v1/shares/{id}"), body).await;
    let used = r.svc.store.usage(LIB);

    let mut config = Config::new(r.dir.path().to_path_buf());
    config.s3 = Some(bucket.config.clone());
    let c = r.clock.clone();
    config.clock = Arc::new(move || c.load(Ordering::SeqCst));
    let svc = Service::open(config).unwrap();
    assert_eq!(svc.store.usage(LIB), used);
    assert!(svc.store.has_blob(LIB, &hex_cid(&keep)).is_some());
    assert!(svc.store.has_blob(LIB, &hex_cid(&gone)).is_none());
    assert_eq!(svc.store.object_len(LIB, "state.abc"), Some(600));
    // The log was rewritten with only what is there.
    let log = std::fs::read_to_string(
        r.dir
            .path()
            .join("libs")
            .join(hex::encode(LIB.0))
            .join("index.log"),
    )
    .unwrap();
    assert_eq!(log.lines().count(), 2, "{log}");
    assert!(!log.contains('-'), "{log}");

    // A week after it expired, the janitor deletes the share and its manifest in the bucket.
    r.clock
        .store(T0 + 60_000 + 7 * 86_400_000, Ordering::SeqCst);
    assert_eq!(svc.sweep().await, 1);
    assert!(bucket.get(&format!("shares/{id}/manifest")).is_none());
    assert!(bucket.refused().is_empty(), "{:?}", bucket.refused());
}
