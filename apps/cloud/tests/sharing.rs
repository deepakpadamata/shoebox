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
use shoebox_core::share::{self, ShareLink};
use shoebox_core::store::*;
use shoebox_core::Hlc;
use shoebox_node::{ImportOptions, LibraryNode, WebShareOptions};
use shoebox_store::{HybridBackend, Library};
use shoebox_testkit::Cluster;
use std::sync::Arc;
use tower::ServiceExt;

struct World {
    _cluster: Cluster,
    node: LibraryNode<Library>,
    svc: Arc<Service>,
    cloud_dir: tempfile::TempDir,
    work: tempfile::TempDir,
    album: AlbumId,
    photos: Vec<PhotoRecord>,
}

fn hlc(ms: u64) -> Hlc {
    Hlc {
        physical_ms: ms,
        counter: 0,
        device: shoebox_core::EndpointId([1; 32]),
    }
}

async fn world(photos_in_album: usize) -> World {
    let cluster = Cluster::new(1).await.unwrap();
    let root = KeyPair::generate();
    let backend = HybridBackend::for_node(cluster.node(0).clone(), cluster.clock().clone());
    let (lib, cap) = backend.create_library(&root.root_public()).await.unwrap();
    let node = LibraryNode::new(
        cluster.node(0).clone(),
        Arc::new(lib),
        cluster.clock().clone(),
    )
    .with_capability(cap);
    node.create_identity(&root, Role::Keeper, "home")
        .await
        .unwrap();

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
    let svc = Service::open(config).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
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
    }
}

async fn get(svc: &Arc<Service>, path: &str) -> (StatusCode, Vec<u8>) {
    let res = router(svc.clone())
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
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
