//! M1-12: the role-based blob fetcher.

mod common;

use common::*;
use iroh_blobs::protocol::{ChunkRanges, ChunkRangesExt};
use shoebox_core::ids::Blake3Hash;
use shoebox_core::model::{BlobKind, BlobRef, Role};
use shoebox_core::store::*;
use shoebox_node::{FetcherConfig, ImportOptions, LibraryNode, NodeEvent, Protection};
use shoebox_store::Library;
use shoebox_testkit::{wait_until, Cluster};
use std::time::Duration;

fn fast() -> FetcherConfig {
    FetcherConfig {
        concurrency: 6,
        tick: Duration::from_millis(300),
        retry: Duration::from_millis(300),
    }
}

async fn photos(n: &LibraryNode<Library>) -> Vec<shoebox_core::model::PhotoRecord> {
    n.store()
        .photos_by_time(TimeRange::ALL, Page::all())
        .await
        .unwrap()
}

async fn has_all(n: &LibraryNode<Library>, kinds: &[BlobKind], expect_count: usize) -> bool {
    let recs = photos(n).await;
    if recs.len() != expect_count {
        return false;
    }
    for r in &recs {
        for kind in kinds {
            let blob = match kind {
                BlobKind::Original => &r.original,
                BlobKind::Preview => &r.preview,
                BlobKind::Thumb => &r.thumb,
            };
            if !n.node().blobs().has(&blob.hash).await.unwrap() {
                return false;
            }
        }
    }
    true
}

fn small_corpus(dir: &std::path::Path) -> (std::path::PathBuf, usize) {
    let out = dir.join("corpus");
    let s = xtask::corpus::generate(&xtask::corpus::Options {
        preset: xtask::corpus::Preset::Small,
        out_dir: out.clone(),
        seed: 11,
        metadata_only: None,
    })
    .unwrap();
    (out.join("photos"), s.photos - s.duplicates)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn keepers_get_every_original_and_viewers_never_download_one() {
    let c = Cluster::with_roles(&[Role::Keeper, Role::Keeper, Role::Viewer])
        .await
        .unwrap();
    let nodes = library_nodes_auto(&c).await;
    let tmp = tempfile::tempdir().unwrap();
    let (folder, unique) = small_corpus(tmp.path());
    nodes[1].start_fetcher(fast());
    nodes[2].start_fetcher(fast());
    let mut viewer_events = c.node(2).events();

    let report = nodes[0]
        .import_paths(&[folder], ImportOptions::default())
        .await
        .unwrap();
    assert_eq!(report.imported.len(), unique);

    let all = [BlobKind::Thumb, BlobKind::Preview, BlobKind::Original];
    wait_until(Duration::from_secs(90), || async {
        has_all(&nodes[1], &all, unique).await
    })
    .await
    .expect("the keeper ends with every original, preview and thumbnail");
    wait_until(Duration::from_secs(90), || async {
        has_all(&nodes[2], &[BlobKind::Thumb, BlobKind::Preview], unique).await
    })
    .await
    .expect("the viewer gets thumbnails and previews");

    // The viewer holds no original, and never downloaded one.
    let originals: Vec<Blake3Hash> = photos(&nodes[2])
        .await
        .iter()
        .map(|p| p.original.hash)
        .collect();
    for h in &originals {
        assert!(
            !nodes[2].node().blobs().has(h).await.unwrap(),
            "viewer must not hold an original"
        );
    }
    while let Ok(res) = tokio::time::timeout(Duration::from_millis(200), viewer_events.recv()).await
    {
        if let Ok(NodeEvent::BlobFetched { hash, .. }) = res {
            assert!(!originals.contains(&hash), "viewer downloaded an original");
        }
    }

    // Fetched blobs are published: every original now has two copies (laptop + keeper).
    wait_until(Duration::from_secs(60), || async {
        for p in photos(&nodes[2]).await {
            if nodes[2].copy_count(&p.id).await.unwrap() != 2 {
                return false;
            }
        }
        true
    })
    .await
    .expect("copy counts reflect the keeper's fetches");
    assert!(
        nodes[2].at_risk(Page::all()).await.unwrap().is_empty(),
        "everything reached the target of 2"
    );
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pausing_stops_fetching_and_resuming_continues() {
    let c = Cluster::with_roles(&[Role::Keeper, Role::Keeper])
        .await
        .unwrap();
    let nodes = library_nodes_auto(&c).await;
    nodes[1].start_fetcher(fast());
    nodes[1].pause_fetcher();
    assert!(nodes[1].fetcher_paused());

    let samples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples");
    let files = [
        samples.join("photo-gps.jpg"),
        samples.join("screenshot.png"),
    ];
    nodes[0]
        .import_paths(&files, ImportOptions::default())
        .await
        .unwrap();
    wait_until(Duration::from_secs(20), || async {
        photos(&nodes[1]).await.len() == 2
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let rec = &photos(&nodes[1]).await[0];
    assert!(
        !nodes[1].node().blobs().has(&rec.thumb.hash).await.unwrap(),
        "paused: nothing is fetched"
    );

    nodes[1].resume_fetcher();
    let all = [BlobKind::Thumb, BlobKind::Preview, BlobKind::Original];
    wait_until(Duration::from_secs(30), || async {
        has_all(&nodes[1], &all, 2).await
    })
    .await
    .expect("resumed: everything arrives");
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fetch_now_gets_an_original_on_demand_and_names_the_source() {
    let c = Cluster::with_roles(&[Role::Keeper, Role::Viewer])
        .await
        .unwrap();
    let nodes = library_nodes_auto(&c).await;
    let samples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples");
    let id = nodes[0]
        .import_paths(&[samples.join("IMG_0001.jpg")], ImportOptions::default())
        .await
        .unwrap()
        .imported[0];
    // The viewer never started a fetcher: this is the explicit "open the original" action.
    wait_until(Duration::from_secs(20), || async {
        nodes[1].store().photo(&id).await.unwrap().is_some()
            && !nodes[1].holders(&id).await.unwrap().is_empty()
    })
    .await
    .unwrap();
    assert!(!nodes[1].node().blobs().has(&id).await.unwrap());

    let mut events = c.node(1).events();
    let source = nodes[1].fetch_now(&id, BlobKind::Original).await.unwrap();
    assert_eq!(
        source.device.0,
        *c.node(0).id().as_bytes(),
        "fetched from the laptop"
    );
    assert!(nodes[1].node().blobs().has(&id).await.unwrap());
    let mut saw = false;
    while let Ok(res) = tokio::time::timeout(Duration::from_millis(300), events.recv()).await {
        if let Ok(NodeEvent::BlobFetched { hash, from }) = res {
            saw |= hash == id && from == c.node(0).id();
        }
    }
    assert!(saw, "a BlobFetched event names the source device");
    // Asking again is free (already local); an unknown photo is an error.
    nodes[1].fetch_now(&id, BlobKind::Original).await.unwrap();
    assert!(nodes[1]
        .fetch_now(&Blake3Hash([9; 32]), BlobKind::Original)
        .await
        .is_err());
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_partly_downloaded_blob_resumes_without_refetching_what_it_has() {
    let c = Cluster::with_roles(&[Role::Keeper, Role::Keeper])
        .await
        .unwrap();
    let nodes = library_nodes_auto(&c).await;

    // A 32 MB "photo" on node 0, registered by hand so no analysis is involved.
    let big: Vec<u8> = (0..32 * 1024 * 1024u32)
        .map(|i| (i.wrapping_mul(2654435761) >> 24) as u8)
        .collect();
    let big_hash = nodes[0]
        .node()
        .blobs()
        .add_bytes(big, Protection::Library)
        .await
        .unwrap();
    let small = |tag: &[u8]| {
        let n0 = nodes[0].node().clone();
        let tag = tag.to_vec();
        async move {
            n0.blobs()
                .add_bytes(tag, Protection::Library)
                .await
                .unwrap()
        }
    };
    let (thumb, preview) = (small(b"thumb").await, small(b"preview").await);
    let blob = |hash: Blake3Hash, size: u64| BlobRef {
        hash,
        size,
        mime: "image/jpeg".into(),
    };
    let rec = {
        let mut p = photo(1, 1);
        p.id = big_hash;
        p.original = blob(big_hash, 32 * 1024 * 1024);
        p.thumb = blob(thumb, 5);
        p.preview = blob(preview, 7);
        p
    };
    nodes[0].store().upsert_photo(rec).await.unwrap();
    nodes[0]
        .publish_holdings(vec![
            (big_hash, BlobKind::Original),
            (thumb, BlobKind::Thumb),
            (preview, BlobKind::Preview),
        ])
        .await
        .unwrap();

    // Node 1 already has the first half (as if a transfer had been interrupted there).
    let ih = iroh_blobs::Hash::from_bytes(big_hash.0);
    let half = 16 * 1024 * 1024u64;
    // An interrupted fetch leaves the partial blob protected (the fetcher does this itself after a
    // failed attempt); hold it here too, since the testkit runs blob GC every 200 ms. Taken *before* the partial data
    // exists so there is no window in which GC could remove it.
    let _partial_protection = nodes[1]
        .node()
        .blobs()
        .raw()
        .tags()
        .temp_tag(iroh_blobs::HashAndFormat {
            hash: ih,
            format: iroh_blobs::BlobFormat::Raw,
        })
        .await
        .unwrap();
    let bao = nodes[0]
        .node()
        .blobs()
        .raw()
        .blobs()
        .export_bao(ih, ChunkRanges::bytes(0..half))
        .bao_to_vec()
        .await
        .unwrap();
    nodes[1]
        .node()
        .blobs()
        .raw()
        .blobs()
        .import_bao_bytes(ih, ChunkRanges::bytes(0..half), bao)
        .await
        .unwrap();
    assert!(
        !nodes[1].node().blobs().has(&big_hash).await.unwrap(),
        "only partly present"
    );

    let mut events = c.node(1).events();
    nodes[1].start_fetcher(fast());
    wait_until(Duration::from_secs(60), || async {
        nodes[1].node().blobs().has(&big_hash).await.unwrap()
    })
    .await
    .expect("the fetcher completes the blob");

    // iroh reports progress as "bytes available" (already local + received), requesting only the
    // missing ranges. A transfer that restarted from scratch would begin near 0; a resumed one
    // begins at the half we already had.
    let mut first = None;
    let mut max_bytes = 0u64;
    while let Ok(res) = tokio::time::timeout(Duration::from_millis(300), events.recv()).await {
        if let Ok(NodeEvent::BlobProgress { hash, bytes, .. }) = res {
            if hash == big_hash {
                first.get_or_insert(bytes);
                max_bytes = max_bytes.max(bytes);
            }
        }
    }
    let first = first.expect("progress was reported for the big blob");
    assert!(
        first >= half,
        "restarted from scratch: first progress was {first}, expected >= {half}"
    );
    assert_eq!(max_bytes, 32 * 1024 * 1024, "and it ran to completion");
    assert_eq!(
        nodes[1].node().blobs().get(&big_hash).await.unwrap().len(),
        32 * 1024 * 1024,
        "and the result is the whole, verified blob"
    );
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn phones_fetch_originals_only_for_pinned_photos() {
    let c = Cluster::with_roles(&[Role::Keeper, Role::Companion])
        .await
        .unwrap();
    let nodes = library_nodes_auto(&c).await;
    // This test is about pins and copies, not the recent window.
    nodes[1].set_keep_rules(0, false).unwrap();
    let samples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples");
    let ids = nodes[0]
        .import_paths(
            &[
                samples.join("photo-gps.jpg"),
                samples.join("screenshot.png"),
            ],
            ImportOptions::default(),
        )
        .await
        .unwrap()
        .imported;
    let (a, b) = (ids[0], ids[1]);
    nodes[1].start_fetcher(fast());

    wait_until(Duration::from_secs(30), || async {
        has_all(&nodes[1], &[BlobKind::Thumb, BlobKind::Preview], 2).await
    })
    .await
    .expect("the phone gets thumbnails and previews of everything");
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(
        !nodes[1].node().blobs().has(&a).await.unwrap()
            && !nodes[1].node().blobs().has(&b).await.unwrap()
    );

    nodes[1].pin_original(&a).await.unwrap();
    assert!(nodes[1].is_pinned(&a) && !nodes[1].is_pinned(&b));
    wait_until(Duration::from_secs(30), || async {
        nodes[1].node().blobs().has(&a).await.unwrap()
    })
    .await
    .expect("a pinned original is fetched");
    // The pin protection is applied right after the download completes.
    wait_until(Duration::from_secs(10), || async {
        nodes[1]
            .node()
            .blobs()
            .protections(&a)
            .await
            .unwrap()
            .contains(&Protection::Pin)
    })
    .await
    .expect("the pinned original is protected from GC");
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(
        !nodes[1].node().blobs().has(&b).await.unwrap(),
        "unpinned originals stay on the keeper"
    );

    // The pin survives in the device's config (it is not replicated).
    let saved = shoebox_node::NodeConfig::load(&nodes[1].node().config().data_dir).unwrap();
    assert_eq!(saved.pinned_originals, vec![a.to_string()]);
    nodes[1].unpin_original(&a).await.unwrap();
    assert!(!nodes[1].is_pinned(&a));
    c.shutdown().await;
}
