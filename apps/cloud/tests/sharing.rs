//! W1-05: sharing an album from a real library node to a real service over a socket, and reading it back the way a
//! viewer would: the link's key opens the manifest, and every blob decrypts to what the library holds.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use shoebox_cloud::{router, Config, Service};
use shoebox_core::clock::Clock;
use shoebox_core::cloud::decrypt_blob;
use shoebox_core::identity::KeyPair;
use shoebox_core::merge::Lww;
use shoebox_core::model::*;
use shoebox_core::share::{self, ShareKey, ShareLink};
use shoebox_core::store::*;
use shoebox_core::Hlc;
use shoebox_node::{CloudPolicy, ImportOptions, LibraryNode, WebShareOptions, WEBVIEW_ALPN};
use shoebox_store::{HybridBackend, Library};
use shoebox_testkit::{wait_until, Cluster};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

mod common;
use common::FakeS3;

struct World {
    _cluster: Cluster,
    node: LibraryNode<Library>,
    svc: Arc<Service>,
    cloud_dir: tempfile::TempDir,
    work: tempfile::TempDir,
    album: AlbumId,
    photos: Vec<PhotoRecord>,
    /// Where the bytes are, when the service keeps them in a bucket.
    bucket: Option<FakeS3>,
}

fn hlc(ms: u64) -> Hlc {
    Hlc {
        physical_ms: ms,
        counter: 0,
        device: shoebox_core::EndpointId([1; 32]),
    }
}

async fn world(photos_in_album: usize) -> World {
    world_as(photos_in_album, Role::Keeper).await
}

async fn world_as(photos_in_album: usize, role: Role) -> World {
    world_with(photos_in_album, role, Bytes::OnDisk).await
}

/// The service keeps the bytes in a bucket; the library uploads to it and readers are sent to it.
async fn world_in_bucket(photos_in_album: usize) -> World {
    world_with(photos_in_album, Role::Keeper, Bytes::FakeBucket).await
}

/// Where the service keeps the bytes.
enum Bytes {
    OnDisk,
    FakeBucket,
    /// A real S3-compatible bucket (see [`real_bucket`]).
    Bucket(shoebox_cloud::s3::S3Config),
}

/// A real bucket named by `SHOEBOX_TEST_S3_ENDPOINT`, `_BUCKET`, `_REGION` (default `us-east-1`),
/// `_ACCESS_KEY_ID` and `_SECRET_ACCESS_KEY`, made if it is not there.
async fn real_bucket() -> Option<shoebox_cloud::s3::S3Config> {
    let var = |n: &str| std::env::var(format!("SHOEBOX_TEST_S3_{n}")).ok();
    let config = shoebox_cloud::s3::S3Config {
        endpoint: var("ENDPOINT")?.parse().unwrap(),
        bucket: var("BUCKET").unwrap_or_else(|| "shoebox-test".into()),
        region: var("REGION").unwrap_or_else(|| "us-east-1".into()),
        access_key: var("ACCESS_KEY_ID")?,
        secret_key: var("SECRET_ACCESS_KEY")?,
        path_style: var("VIRTUAL_HOST").is_none(),
    };
    use rusty_s3::S3Action;
    let style = if config.path_style {
        rusty_s3::UrlStyle::Path
    } else {
        rusty_s3::UrlStyle::VirtualHost
    };
    let bucket = rusty_s3::Bucket::new(
        config.endpoint.clone(),
        style,
        config.bucket.clone(),
        config.region.clone(),
    )
    .unwrap();
    let creds = rusty_s3::Credentials::new(config.access_key.clone(), config.secret_key.clone());
    let url = rusty_s3::actions::CreateBucket::new(&bucket, &creds).sign(Duration::from_secs(60));
    let res = common::http()
        .put(url)
        .header("content-length", 0)
        .send()
        .await
        .unwrap();
    assert!(
        res.status().is_success() || res.status().as_u16() == 409,
        "making the bucket: {}",
        res.status()
    );
    Some(config)
}

async fn world_with(photos_in_album: usize, role: Role, bytes: Bytes) -> World {
    let cluster = Cluster::with_roles(&[role, role]).await.unwrap();
    if matches!(bytes, Bytes::Bucket(_)) {
        // A real bucket checks presigned times against real time.
        let now = shoebox_cloud::system_clock()() as u64;
        cluster.clock().advance(now - cluster.clock().now_ms());
    }
    let root = KeyPair::generate();
    let backend = HybridBackend::for_node(cluster.node(0).clone(), cluster.clock().clone());
    let (lib, cap) = backend.create_library(&root.root_public()).await.unwrap();
    let node = LibraryNode::new(
        cluster.node(0).clone(),
        Arc::new(lib),
        cluster.clock().clone(),
    )
    .with_capability(cap);
    node.create_identity(&root, role, "home").await.unwrap();

    // Photos from the fixture corpus, imported the normal way (thumbnails and previews are made).
    let work = tempfile::tempdir().unwrap();
    let out = work.path().join("corpus");
    xtask::corpus::generate(&xtask::corpus::Options {
        preset: xtask::corpus::Preset::Small,
        out_dir: out.clone(),
        seed: 11,
        metadata_only: None,
    })
    .unwrap();
    node.import_paths(&[out.join("photos")], ImportOptions::default())
        .await
        .unwrap();
    let mut all = node
        .store()
        .photos_by_time(TimeRange::ALL, Page::all())
        .await
        .unwrap();
    all.retain(|p| !p.original.mime.starts_with("video/"));
    assert!(
        all.len() >= photos_in_album + 2,
        "the corpus is too small: {}",
        all.len()
    );
    let photos: Vec<PhotoRecord> = all.into_iter().take(photos_in_album + 2).collect();

    let album = AlbumId([7; 16]);
    let mut items = shoebox_core::merge::OrSet::new();
    for p in photos.iter().take(photos_in_album) {
        items.add(p.id, hlc(2));
    }
    node.store()
        .upsert_album(AlbumMeta {
            id: album,
            name: Lww::new("Lisbon".into(), hlc(1)),
            cover: Lww::new(None, hlc(1)),
            items,
        })
        .await
        .unwrap();

    // The service, on a real socket, with the library's clock so signed times agree.
    let cloud_dir = tempfile::tempdir().unwrap();
    let mut config = Config::new(cloud_dir.path().to_path_buf());
    let clock = cluster.clock().clone();
    config.clock = Arc::new(move || clock.now_ms() as i64);
    let bucket = match bytes {
        Bytes::OnDisk => None,
        Bytes::FakeBucket => {
            let clock = cluster.clock().clone();
            let b = common::start(Arc::new(move || clock.now_ms() as i64)).await;
            config.s3 = Some(b.config.clone());
            Some(b)
        }
        Bytes::Bucket(c) => {
            config.s3 = Some(c);
            None
        }
    };
    // For the by-hand run below: serve the built viewer page and listen on a known port.
    config.viewer_dir = std::env::var("SHOEBOX_DEMO_VIEWER").ok().map(Into::into);
    let port = std::env::var("SHOEBOX_DEMO_PORT").unwrap_or_else(|_| "0".into());
    let svc = Service::open(config).unwrap();
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(svc.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    node.set_cloud_url(&format!("http://{addr}")).await.unwrap();

    World {
        _cluster: cluster,
        node,
        svc,
        cloud_dir,
        work,
        album,
        photos,
        bucket,
    }
}

/// A GET as the viewer makes it, following a redirect to the bucket.
async fn get(svc: &Arc<Service>, path: &str) -> (StatusCode, Vec<u8>) {
    let res = router(svc.clone())
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    if status == StatusCode::FOUND {
        let to = res.headers()["location"].to_str().unwrap().to_string();
        let res = common::http().get(to).send().await.unwrap();
        let status = StatusCode::from_u16(res.status().as_u16()).unwrap();
        return (status, res.bytes().await.unwrap().to_vec());
    }
    (
        status,
        res.into_body().collect().await.unwrap().to_bytes().to_vec(),
    )
}

/// Open a share the way the viewer page does: from the link alone.
async fn open_share(w: &World, link: &str) -> (share::Header, share::Manifest) {
    let fragment = link.split_once('#').unwrap().1;
    let l = ShareLink::parse(fragment).expect("a link that parses");
    let id = hex::encode(l.share_id().unwrap());
    let (s, sealed) = get(&w.svc, &format!("/s/{id}/m")).await;
    assert_eq!(s, StatusCode::OK);
    share::open(&sealed, &l.key, &l.key.signer().public_bytes()).expect("the link's key opens it")
}

async fn blob(w: &World, share_id: &[u8; 16], b: &share::CloudBlob) -> Vec<u8> {
    let (s, bytes) = get(
        &w.svc,
        &format!("/s/{}/b/{}", hex::encode(share_id), hex::encode(b.cid.0)),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        blake3::hash(&bytes).as_bytes(),
        &b.cid.0,
        "a blob matches its id"
    );
    decrypt_blob(&b.key(), &bytes).expect("the manifest's key opens the blob")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_album_is_shared_and_opens_from_the_link_alone() {
    let w = world(6).await;
    let share = w
        .node
        .create_web_share(&w.album, WebShareOptions::default())
        .await
        .unwrap();
    assert_eq!(
        (share.items, share.skipped, share.withheld, share.version),
        (6, 0, 0, 0)
    );
    let link = w.node.web_share_link(&share).await.unwrap();
    assert!(link.contains("/a#c."));

    let (header, manifest) = open_share(&w, &link).await;
    assert_eq!(header.share_id, share.share_id);
    assert_eq!(manifest.album_name, "Lisbon");
    assert_eq!(manifest.items.len(), 6);
    let blobs = w.node.node().blobs();
    for item in &manifest.items {
        assert!(item.original.is_none(), "no full-size unless asked");
        // Which photo is this? The one taken at that time.
        let p = w
            .photos
            .iter()
            .find(|p| p.taken_at == item.taken && p.exif.width == item.width)
            .expect("an item for each photo");
        assert_eq!(
            blob(&w, &share.share_id, &item.thumb).await,
            blobs.get(&p.thumb.hash).await.unwrap().to_vec()
        );
        assert_eq!(
            blob(&w, &share.share_id, &item.preview).await,
            blobs.get(&p.preview.hash).await.unwrap().to_vec()
        );
    }
    // The library remembers the link, so any device can show it again.
    assert_eq!(w.node.web_shares(&w.album).await.unwrap(), vec![share]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_service_never_holds_a_photo() {
    let w = world(4).await;
    w.node
        .create_web_share(
            &w.album,
            WebShareOptions {
                full_size: true,
                expires_ms: None,
            },
        )
        .await
        .unwrap();
    let blobs = w.node.node().blobs();
    let mut needles = vec![];
    for p in w.photos.iter().take(4) {
        for hash in [&p.original.hash, &p.preview.hash, &p.thumb.hash] {
            let b = blobs.get(hash).await.unwrap();
            if b.len() > 600 {
                needles.push(b[b.len() / 2..b.len() / 2 + 256].to_vec());
            }
        }
    }
    assert!(needles.len() >= 8);
    let mut stack = vec![w.cloud_dir.path().to_path_buf()];
    let mut files = 0;
    while let Some(p) = stack.pop() {
        for e in std::fs::read_dir(&p).unwrap() {
            let path = e.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let bytes = std::fs::read(&path).unwrap();
            files += 1;
            for n in &needles {
                assert!(
                    !bytes.windows(n.len()).any(|w| w == n.as_slice()),
                    "plaintext in {}",
                    path.display()
                );
            }
        }
    }
    assert!(files > 8);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_size_originals_are_uploaded_without_their_location() {
    let w = world(6).await;
    let share = w
        .node
        .create_web_share(
            &w.album,
            WebShareOptions {
                full_size: true,
                expires_ms: None,
            },
        )
        .await
        .unwrap();
    let link = w.node.web_share_link(&share).await.unwrap();
    let (_, manifest) = open_share(&w, &link).await;
    let blobs = w.node.node().blobs();
    let (mut with_place, mut sent) = (0, 0);
    for item in &manifest.items {
        let p = w
            .photos
            .iter()
            .find(|p| p.taken_at == item.taken && p.exif.width == item.width)
            .unwrap();
        let Some(original) = &item.original else {
            // Withheld only for a format whose location cannot be removed.
            assert!(
                p.exif.gps.is_some(),
                "an original without a place must be sent"
            );
            continue;
        };
        sent += 1;
        let got = blob(&w, &share.share_id, original).await;
        let local = blobs.get(&p.original.hash).await.unwrap().to_vec();
        if p.exif.gps.is_some() {
            with_place += 1;
            assert_ne!(got, local, "a photo with a place is not sent as it is");
            let stripped = shoebox_media::strip_location(&local).unwrap();
            assert_eq!(got, stripped.bytes);
        } else {
            assert_eq!(got, local);
        }
    }
    assert!(sent > 0);
    assert_eq!(share.withheld as usize + sent, 6);
    eprintln!(
        "{sent} originals sent, {with_place} of them had a place removed, {} withheld",
        share.withheld
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_changed_album_is_refreshed_and_an_unchanged_one_is_left_alone() {
    let w = world(4).await;
    let share = w
        .node
        .create_web_share(&w.album, WebShareOptions::default())
        .await
        .unwrap();
    let link = w.node.web_share_link(&share).await.unwrap();

    let r = w.node.refresh_web_shares().await.unwrap();
    assert_eq!((r.refreshed, r.current), (0, 1), "{:?}", r.failed);

    // Another photo goes into the album.
    w.node
        .store()
        .album_add(&w.album, &[w.photos[4].id])
        .await
        .unwrap();
    let r = w.node.refresh_web_shares().await.unwrap();
    assert_eq!((r.refreshed, r.current), (1, 0), "{:?}", r.failed);
    // The same link now shows five photos, at version 1.
    let (header, manifest) = open_share(&w, &link).await;
    assert_eq!(manifest.items.len(), 5);
    assert_eq!(header.manifest_version, 1);
    // A photo deleted from the library drops out.
    w.node.store().mark_deleted(&w.photos[0].id).await.unwrap();
    assert_eq!(w.node.refresh_web_shares().await.unwrap().refreshed, 1);
    let (header, manifest) = open_share(&w, &link).await;
    assert_eq!((manifest.items.len(), header.manifest_version), (4, 2));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_revoked_link_stops_at_once_and_a_rotated_one_replaces_it() {
    let w = world(3).await;
    let share = w
        .node
        .create_web_share(&w.album, WebShareOptions::default())
        .await
        .unwrap();
    let id = hex::encode(share.share_id);
    assert_eq!(get(&w.svc, &format!("/s/{id}/m")).await.0, StatusCode::OK);

    let new = w
        .node
        .rotate_web_share(&w.album, &share.share_id)
        .await
        .unwrap();
    assert_ne!(new.share_id, share.share_id);
    assert_ne!(new.key, share.key);
    assert_eq!(
        get(&w.svc, &format!("/s/{id}/m")).await.0,
        StatusCode::NOT_FOUND
    );
    let link = w.node.web_share_link(&new).await.unwrap();
    assert_eq!(open_share(&w, &link).await.1.items.len(), 3);
    assert_eq!(w.node.web_shares(&w.album).await.unwrap().len(), 1);

    w.node
        .revoke_web_share(&w.album, &new.share_id)
        .await
        .unwrap();
    assert_eq!(
        get(&w.svc, &format!("/s/{}/m", hex::encode(new.share_id)))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert!(w.node.web_shares(&w.album).await.unwrap().is_empty());
    // Revoking again is not an error.
    w.node
        .revoke_web_share(&w.album, &new.share_id)
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn blobs_already_uploaded_are_not_uploaded_again_and_certificates_reach_the_service() {
    let w = world(4).await;
    w.node
        .create_web_share(&w.album, WebShareOptions::default())
        .await
        .unwrap();
    let used = w.svc.store.usage(w.node.person_id());
    assert!(used > 0);
    // A second link to the same album seals the same blobs to the same bytes: only the manifest is new.
    w.node
        .create_web_share(&w.album, WebShareOptions::default())
        .await
        .unwrap();
    let after = w.svc.store.usage(w.node.person_id());
    assert!(
        after - used < 4096,
        "the second share added {} bytes",
        after - used
    );
    w.node.push_cloud_certs().await.unwrap();
    let _ = &w.work;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_library_without_the_cloud_enabled_says_so() {
    let w = world(2).await;
    // A library whose secret was never set (made before the cloud existed) is told what to do.
    w.node
        .store()
        .put_setting(shoebox_node::CLOUD_SECRET_KEY, vec![])
        .await
        .unwrap();
    let err = w
        .node
        .create_web_share(&w.album, WebShareOptions::default())
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("recovery phrase"), "{err:#}");
}

/// Serve a real share and its viewer page for looking at in a browser. Not part of the suite:
/// `SHOEBOX_DEMO_VIEWER=apps/web-viewer/dist SHOEBOX_DEMO_PORT=18090 cargo test -p shoebox-cloud --test sharing demo -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "runs until stopped"]
async fn demo_serve_a_share() {
    // With SHOEBOX_TEST_S3_* set, the bytes go to that bucket and the page is sent there for them.
    let w = match real_bucket().await {
        Some(c) => world_with(10, Role::Keeper, Bytes::Bucket(c)).await,
        None => world(10).await,
    };
    let share = w
        .node
        .create_web_share(
            &w.album,
            WebShareOptions {
                full_size: true,
                expires_ms: None,
            },
        )
        .await
        .unwrap();
    println!("LINK {}", w.node.web_share_link(&share).await.unwrap());
    tokio::time::sleep(std::time::Duration::from_secs(1800)).await;
}

/// One request over `shoebox/webview/0`, as a browser sends it: (status, file size, bytes).
async fn ask(conn: &iroh::endpoint::Connection, req: Vec<u8>) -> (u8, u64, Vec<u8>) {
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(&req).await.unwrap();
    send.finish().unwrap();
    let reply = recv.read_to_end(64 * 1024 * 1024).await.unwrap();
    match reply.split_first() {
        Some((0, rest)) => (
            0,
            u64::from_le_bytes(rest[..8].try_into().unwrap()),
            rest[8..].to_vec(),
        ),
        Some((s, _)) => (*s, 0, vec![]),
        None => panic!("an empty reply"),
    }
}

fn blob_request(id: &[u8; 16], cid: &[u8; 32], offset: u64, len: u32) -> Vec<u8> {
    let mut r = vec![2u8];
    r.extend_from_slice(id);
    r.extend_from_slice(cid);
    r.extend_from_slice(&offset.to_le_bytes());
    r.extend_from_slice(&len.to_le_bytes());
    r
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_live_link_is_served_by_the_device_and_stops_when_revoked() {
    let w = world(4).await;
    let share = w
        .node
        .create_live_share(&w.album, WebShareOptions::default())
        .await
        .unwrap();
    let link = w.node.live_share_link(&share).await.unwrap();
    let l = ShareLink::parse(link.split_once('#').unwrap().1).expect("a Live link parses");
    assert_eq!(l.mode, share::LinkMode::Live);
    assert_eq!(l.id(), Some(share.share_id));
    assert_eq!(
        l.servers()[0],
        w.node.device().0,
        "the link names this device"
    );
    // The service holds nothing for it.
    assert_eq!(
        get(&w.svc, &format!("/s/{}/m", hex::encode(share.share_id)))
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    // Another node, standing in for the browser, asks the device directly.
    let addr = w._cluster.node(0).addr();
    let conn = w
        ._cluster
        .node(1)
        .endpoint()
        .connect(addr, WEBVIEW_ALPN)
        .await
        .expect("the device answers the webview protocol");
    let mut req = vec![1u8];
    req.extend_from_slice(&share.share_id);
    let (status, total, sealed) = ask(&conn, req.clone()).await;
    assert_eq!((status, total), (0, sealed.len() as u64));
    let (_, manifest) = share::open(&sealed, &l.key, &l.key.signer().public_bytes())
        .expect("the link's key opens the manifest");
    assert_eq!(manifest.album_name, "Lisbon");
    assert_eq!(manifest.items.len(), 4);

    // A thumbnail, fetched in pieces of 1000 bytes, matches its id and decrypts to the library's thumbnail.
    let item = &manifest.items[0];
    let mut bytes = vec![];
    loop {
        let (s, total, part) = ask(
            &conn,
            blob_request(&share.share_id, &item.thumb.cid.0, bytes.len() as u64, 1000),
        )
        .await;
        assert_eq!(s, 0);
        bytes.extend_from_slice(&part);
        if bytes.len() as u64 >= total {
            break;
        }
    }
    assert_eq!(blake3::hash(&bytes).as_bytes(), &item.thumb.cid.0);
    let plain = decrypt_blob(&item.thumb.key(), &bytes).unwrap();
    let p = w
        .photos
        .iter()
        .find(|p| p.taken_at == item.taken && p.exif.width == item.width)
        .unwrap();
    assert_eq!(
        plain,
        w.node
            .node()
            .blobs()
            .get(&p.thumb.hash)
            .await
            .unwrap()
            .to_vec()
    );

    // It serves this share and nothing else: another id, a blob the share does not list, a bad request.
    let mut other = vec![1u8];
    other.extend_from_slice(&[9; 16]);
    assert_eq!(ask(&conn, other).await.0, 1);
    assert_eq!(
        ask(&conn, blob_request(&share.share_id, &[5; 32], 0, 10))
            .await
            .0,
        1
    );
    assert_eq!(ask(&conn, vec![9]).await.0, 4);
    let mut traversal = blob_request(&share.share_id, &[0; 32], 0, 10);
    traversal.splice(17..49, *b"../meta.........................");
    assert_eq!(ask(&conn, traversal).await.0, 1);

    // Revoking stops it at once and removes the ciphertext.
    w.node
        .revoke_live_share(&w.album, &share.share_id)
        .await
        .unwrap();
    assert_eq!(ask(&conn, req).await.0, 1);
    assert!(!w.node.serves_live_share(&share.share_id));
    assert!(w.node.web_live_shares(&w.album).await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_live_link_expires_and_follows_its_album() {
    let w = world(3).await;
    let expires = w._cluster.clock().now_ms() as i64 + 60_000;
    let share = w
        .node
        .create_live_share(
            &w.album,
            WebShareOptions {
                full_size: false,
                expires_ms: Some(expires),
            },
        )
        .await
        .unwrap();
    let addr = w._cluster.node(0).addr();
    let conn = w
        ._cluster
        .node(1)
        .endpoint()
        .connect(addr, WEBVIEW_ALPN)
        .await
        .unwrap();
    let mut req = vec![1u8];
    req.extend_from_slice(&share.share_id);
    assert_eq!(ask(&conn, req.clone()).await.0, 0);

    // Nothing changed: nothing to do. Add a photo: the device seals again and the version moves on.
    let r = w.node.sync_live_shares(false).await.unwrap();
    assert_eq!((r.refreshed, r.current), (0, 1));
    let mut items = shoebox_core::merge::OrSet::new();
    for p in w.photos.iter().take(4) {
        items.add(p.id, hlc(3));
    }
    w.node
        .store()
        .upsert_album(AlbumMeta {
            id: w.album,
            name: Lww::new("Lisbon".into(), hlc(1)),
            cover: Lww::new(None, hlc(1)),
            items,
        })
        .await
        .unwrap();
    assert!(w
        .node
        .web_share_is_stale(&w.node.web_live_shares(&w.album).await.unwrap()[0])
        .await
        .unwrap());
    let r = w.node.sync_live_shares(false).await.unwrap();
    assert_eq!(r.refreshed, 1, "{:?}", r.failed);
    let (_, _, sealed) = ask(&conn, req.clone()).await;
    let key = ShareKey(share.key);
    let (header, manifest) = share::open(&sealed, &key, &key.signer().public_bytes()).unwrap();
    assert_eq!((header.manifest_version, manifest.items.len()), (1, 4));

    // After the date it says so, and does not serve.
    w._cluster.clock().advance(61_000);
    assert_eq!(ask(&conn, req).await.0, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_device_without_the_folder_serves_a_live_link_only_when_asked_to() {
    let w = world(3).await;
    let share = w
        .node
        .create_live_share(&w.album, WebShareOptions::default())
        .await
        .unwrap();
    assert!(w.node.serves_live_share(&share.share_id));

    // Another device of the library has the record (it is a library setting) but not the folder; the same
    // situation as here once the folder is gone.
    let dir = w
        .node
        .node()
        .config()
        .data_dir
        .join("live")
        .join(hex::encode(share.share_id));
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(!w.node.serves_live_share(&share.share_id));

    let r = w.node.sync_live_shares(false).await.unwrap();
    assert_eq!((r.adopted, r.refreshed, r.current), (0, 0, 0));
    assert!(
        !w.node.serves_live_share(&share.share_id),
        "not unless asked"
    );

    let r = w.node.sync_live_shares(true).await.unwrap();
    assert_eq!((r.adopted, r.failed.len()), (1, 0), "{:?}", r.failed);
    assert!(w.node.serves_live_share(&share.share_id));

    // It answers with a manifest the link's key opens.
    let addr = w._cluster.node(0).addr();
    let conn = w
        ._cluster
        .node(1)
        .endpoint()
        .connect(addr, WEBVIEW_ALPN)
        .await
        .unwrap();
    let mut req = vec![1u8];
    req.extend_from_slice(&share.share_id);
    let (status, _, sealed) = ask(&conn, req).await;
    assert_eq!(status, 0);
    let key = ShareKey(share.key);
    let (_, m) = share::open(&sealed, &key, &key.signer().public_bytes()).unwrap();
    assert_eq!(m.items.len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_cloud_keeper_uploads_once_and_restores_every_blob_checked() {
    let w = world(4).await;
    let policy = CloudPolicy {
        originals: true,
        byte_budget: None,
    };
    let r = w.node.cloud_backup(&policy).await.unwrap();
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    assert!(r.uploaded > 0 && r.left == 0);
    let first = r.uploaded;

    // Nothing changed: nothing is uploaded or sealed again.
    let again = w.node.cloud_backup(&policy).await.unwrap();
    assert_eq!((again.uploaded, again.already), (0, first), "{again:?}");

    // The service holds the library state and catalog, and no photo.
    let stored = std::fs::read_dir(w.cloud_dir.path()).unwrap().count();
    assert!(stored > 0);
    let p = &w.photos[0];
    let original = w.node.node().blobs().get(&p.original.hash).await.unwrap();

    // Lose a photo's blobs from this device, then restore.
    let blobs = w.node.node().blobs();
    let lost = [p.original.hash, p.thumb.hash, p.preview.hash];
    for h in lost {
        blobs.remove(&h).await.unwrap();
    }
    wait_until(Duration::from_secs(30), || async {
        let mut gone = true;
        for h in lost {
            gone &= !blobs.has(&h).await.unwrap();
        }
        gone
    })
    .await
    .expect("the store collects released blobs");
    let r = w.node.cloud_restore().await.unwrap();
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    assert!(r.state_imported);
    assert!(r.restored >= 3, "{r:?}");
    assert_eq!(blobs.get(&p.original.hash).await.unwrap(), original);
    for h in [p.thumb.hash, p.preview.hash] {
        assert!(blobs.has(&h).await.unwrap());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_small_budget_stops_and_the_next_run_carries_on() {
    let w = world(4).await;
    let first = w
        .node
        .cloud_backup(&CloudPolicy {
            originals: false,
            byte_budget: Some(1),
        })
        .await
        .unwrap();
    assert!(first.uploaded >= 1 && first.left > 0, "{first:?}");
    let rest = w
        .node
        .cloud_backup(&CloudPolicy {
            originals: false,
            byte_budget: None,
        })
        .await
        .unwrap();
    assert_eq!(rest.left, 0);
    assert!(rest.failed.is_empty(), "{:?}", rest.failed);
    // Only thumbnails and previews went up without the originals option.
    let all = w
        .node
        .store()
        .photos_by_time(TimeRange::ALL, Page::all())
        .await
        .unwrap()
        .len() as u32;
    assert!(
        first.uploaded + rest.uploaded <= all * 2,
        "no originals went up"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_encrypted_copy_lets_a_phone_free_space_and_says_it_was_the_only_one() {
    let w = world_as(2, Role::Companion).await;
    w.node.set_keep_rules(0, false).unwrap();
    // No other device and nothing in the cloud yet: nothing may go.
    let none = w.node.free_up_space(true).await.unwrap();
    assert!(none.photos.is_empty(), "{none:?}");

    let r = w
        .node
        .cloud_backup(&CloudPolicy {
            originals: true,
            byte_budget: None,
        })
        .await
        .unwrap();
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    let dry = w.node.free_up_space(true).await.unwrap();
    assert!(!dry.photos.is_empty());
    assert_eq!(
        dry.cloud_only, dry.photos,
        "the cloud was the only other copy"
    );

    let done = w.node.free_up_space(false).await.unwrap();
    assert_eq!(done.photos, dry.photos);
    // The original is gone from this device (collection reclaims it shortly after) and the restore brings it back.
    let id = done.photos[0];
    assert!(w
        .node
        .node()
        .blobs()
        .protections(&id)
        .await
        .unwrap()
        .is_empty());
    wait_until(Duration::from_secs(30), || async {
        !w.node.node().blobs().has(&id).await.unwrap()
    })
    .await
    .expect("the store collects it");
    let r = w.node.cloud_restore().await.unwrap();
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    assert!(w.node.node().blobs().has(&id).await.unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_link_changes_what_it_offers_and_keeps_its_address() {
    let w = world(3).await;
    let share = w
        .node
        .create_web_share(&w.album, WebShareOptions::default())
        .await
        .unwrap();
    let link = w.node.web_share_link(&share).await.unwrap();
    assert!(open_share(&w, &link).await.1.items[0].original.is_none());

    let new = w
        .node
        .update_web_share(
            &w.album,
            &share.share_id,
            WebShareOptions {
                full_size: true,
                expires_ms: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        (new.share_id, new.key, new.version),
        (share.share_id, share.key, 1)
    );
    assert_eq!(
        w.node.web_share_link(&new).await.unwrap(),
        link,
        "same address"
    );
    let (h, m) = open_share(&w, &link).await;
    assert_eq!(h.manifest_version, 1);
    assert!(
        m.items.iter().any(|i| i.original.is_some()),
        "full size is offered now"
    );
    assert_eq!(w.node.web_shares(&w.album).await.unwrap(), vec![new]);
}

// ---- the service keeps the bytes in a bucket ----

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn from_a_bucket_an_album_opens_from_the_link_alone_and_stops_when_revoked() {
    let w = world_in_bucket(4).await;
    let bucket = w.bucket.clone().unwrap();
    let share = w
        .node
        .create_web_share(
            &w.album,
            WebShareOptions {
                full_size: true,
                expires_ms: None,
            },
        )
        .await
        .unwrap();
    assert_eq!((share.items, share.skipped), (4, 0));
    let link = w.node.web_share_link(&share).await.unwrap();
    let (_, manifest) = open_share(&w, &link).await;
    let blobs = w.node.node().blobs();
    for item in &manifest.items {
        let p = w
            .photos
            .iter()
            .find(|p| p.taken_at == item.taken && p.exif.width == item.width)
            .unwrap();
        assert_eq!(
            blob(&w, &share.share_id, &item.preview).await,
            blobs.get(&p.preview.hash).await.unwrap().to_vec()
        );
        assert!(item.original.is_some());
    }

    // The bytes are in the bucket, sealed; the data directory holds no blob at all.
    let keys = bucket.keys();
    let lib = hex::encode(w.node.person_id().0);
    let in_bucket = keys
        .iter()
        .filter(|k| k.starts_with(&format!("libs/{lib}/blobs/")))
        .count();
    assert!(in_bucket >= 8, "{keys:?}");
    let mut needles = vec![];
    for p in w.photos.iter().take(4) {
        for hash in [&p.original.hash, &p.preview.hash, &p.thumb.hash] {
            let b = blobs.get(hash).await.unwrap();
            if b.len() > 600 {
                needles.push(b[b.len() / 2..b.len() / 2 + 256].to_vec());
            }
        }
    }
    for k in &keys {
        let bytes = bucket.get(k).unwrap();
        for n in &needles {
            assert!(
                !bytes.windows(n.len()).any(|x| x == n.as_slice()),
                "plaintext in {k}"
            );
        }
    }
    let mut stack = vec![w.cloud_dir.path().to_path_buf()];
    while let Some(p) = stack.pop() {
        for e in std::fs::read_dir(&p).unwrap() {
            let path = e.unwrap().path();
            if path.is_dir() {
                assert!(
                    !path.ends_with("blobs") || std::fs::read_dir(&path).unwrap().next().is_none()
                );
                stack.push(path);
            } else {
                assert!(
                    std::fs::metadata(&path).unwrap().len() < 64 * 1024,
                    "{}",
                    path.display()
                );
            }
        }
    }

    // Revoked: the manifest leaves the bucket and the link answers "gone".
    let id = hex::encode(share.share_id);
    w.node
        .revoke_web_share(&w.album, &share.share_id)
        .await
        .unwrap();
    assert!(bucket.get(&format!("shares/{id}/manifest")).is_none());
    assert_eq!(
        get(&w.svc, &format!("/s/{id}/m")).await.0,
        StatusCode::NOT_FOUND
    );
    assert!(bucket.refused().is_empty(), "{:?}", bucket.refused());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn from_a_bucket_the_cloud_keeper_uploads_once_and_restores_every_blob_checked() {
    let w = world_in_bucket(4).await;
    let bucket = w.bucket.clone().unwrap();
    let policy = CloudPolicy {
        originals: true,
        byte_budget: None,
    };
    let r = w.node.cloud_backup(&policy).await.unwrap();
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    assert!(r.uploaded > 0 && r.left == 0);
    let again = w.node.cloud_backup(&policy).await.unwrap();
    assert_eq!(
        (again.uploaded, again.already),
        (0, r.uploaded),
        "{again:?}"
    );
    let lib = hex::encode(w.node.person_id().0);
    let objects: Vec<String> = bucket
        .keys()
        .into_iter()
        .filter(|k| k.starts_with(&format!("libs/{lib}/objects/")))
        .collect();
    assert_eq!(objects.len(), 2, "the catalog and the state: {objects:?}");

    let p = &w.photos[0];
    let blobs = w.node.node().blobs();
    let original = blobs.get(&p.original.hash).await.unwrap();
    let lost = [p.original.hash, p.thumb.hash, p.preview.hash];
    for h in lost {
        blobs.remove(&h).await.unwrap();
    }
    wait_until(Duration::from_secs(30), || async {
        let mut gone = true;
        for h in lost {
            gone &= !blobs.has(&h).await.unwrap();
        }
        gone
    })
    .await
    .expect("the store collects released blobs");
    let r = w.node.cloud_restore().await.unwrap();
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    assert!(r.state_imported && r.restored >= 3, "{r:?}");
    assert_eq!(blobs.get(&p.original.hash).await.unwrap(), original);
    assert!(bucket.refused().is_empty(), "{:?}", bucket.refused());
}

/// The same against a real S3-compatible bucket (MinIO, R2, B2...): signatures checked by someone else's code.
/// `SHOEBOX_TEST_S3_ENDPOINT=http://127.0.0.1:9000 SHOEBOX_TEST_S3_ACCESS_KEY_ID=... SHOEBOX_TEST_S3_SECRET_ACCESS_KEY=...
/// cargo test -p shoebox-cloud --test sharing real_bucket -- --ignored`
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs a real bucket"]
async fn with_a_real_bucket_a_link_opens_and_the_library_restores() {
    let Some(config) = real_bucket().await else {
        panic!("set SHOEBOX_TEST_S3_ENDPOINT, _ACCESS_KEY_ID and _SECRET_ACCESS_KEY");
    };
    let w = world_with(4, Role::Keeper, Bytes::Bucket(config)).await;
    let share = w
        .node
        .create_web_share(
            &w.album,
            WebShareOptions {
                full_size: true,
                expires_ms: None,
            },
        )
        .await
        .unwrap();
    let link = w.node.web_share_link(&share).await.unwrap();
    let (_, manifest) = open_share(&w, &link).await;
    let blobs = w.node.node().blobs();
    for item in &manifest.items {
        let p = w
            .photos
            .iter()
            .find(|p| p.taken_at == item.taken && p.exif.width == item.width)
            .unwrap();
        assert_eq!(
            blob(&w, &share.share_id, &item.thumb).await,
            blobs.get(&p.thumb.hash).await.unwrap().to_vec()
        );
    }

    let policy = CloudPolicy {
        originals: true,
        byte_budget: None,
    };
    let r = w.node.cloud_backup(&policy).await.unwrap();
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    let p = &w.photos[1];
    let original = blobs.get(&p.original.hash).await.unwrap();
    blobs.remove(&p.original.hash).await.unwrap();
    wait_until(Duration::from_secs(30), || async {
        !blobs.has(&p.original.hash).await.unwrap()
    })
    .await
    .expect("the store collects released blobs");
    let r = w.node.cloud_restore().await.unwrap();
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    assert_eq!(blobs.get(&p.original.hash).await.unwrap(), original);

    let id = hex::encode(share.share_id);
    w.node
        .revoke_web_share(&w.album, &share.share_id)
        .await
        .unwrap();
    assert_eq!(
        get(&w.svc, &format!("/s/{id}/m")).await.0,
        StatusCode::NOT_FOUND
    );
}
