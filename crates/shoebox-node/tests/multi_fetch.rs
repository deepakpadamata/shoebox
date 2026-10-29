//! A-099: a blob comes from several holders at once (large blobs, in pieces) or from the next
//! holder when one cannot provide it (every blob).

use shoebox_node::{Node, NodeConfig, Protection};
use std::time::{Duration, Instant};
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(60);

async fn open(dir: &tempfile::TempDir, name: &str) -> Node {
    Node::open(NodeConfig::local_only(dir.path().join(name), name))
        .await
        .expect("node opens")
}

fn data(len: usize, seed: u32) -> Vec<u8> {
    (0..len as u32)
        .map(|i| (i.wrapping_mul(2654435761).wrapping_add(seed) >> 13) as u8)
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_large_blob_comes_from_every_holder_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b, c) = (
        open(&dir, "a").await,
        open(&dir, "b").await,
        open(&dir, "c").await,
    );
    let bytes = data(24 * 1024 * 1024 + 12_345, 1); // not a whole number of pieces
    let hash = a
        .blobs()
        .add_bytes(bytes.clone(), Protection::Library)
        .await
        .unwrap();
    b.blobs()
        .add_bytes(bytes.clone(), Protection::Library)
        .await
        .unwrap();
    c.add_peer_addr(a.addr());
    c.add_peer_addr(b.addr());

    let report = timeout(
        WAIT,
        c.fetch_blob_from(
            &[a.id(), b.id()],
            &hash,
            Some(bytes.len() as u64),
            Protection::Library,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        c.blobs().get(&hash).await.unwrap().as_ref(),
        bytes.as_slice()
    );
    assert_eq!(
        c.blobs().protections(&hash).await.unwrap(),
        vec![Protection::Library]
    );
    assert_eq!(
        report.sources.len(),
        2,
        "both holders sent pieces: {report:?}"
    );
    let sent: u64 = report.sources.iter().map(|(_, n)| n).sum();
    assert!(
        sent >= bytes.len() as u64 && sent < bytes.len() as u64 + 4 * 1024 * 1024,
        "about one copy's worth in all, not two: {sent}"
    );
    for n in [&a, &b, &c] {
        n.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn holders_that_cannot_help_are_left_out_without_waiting_for_them() {
    let dir = tempfile::tempdir().unwrap();
    let (a, empty, c) = (
        open(&dir, "a").await,
        open(&dir, "empty").await,
        open(&dir, "c").await,
    );
    let gone = open(&dir, "gone").await;
    let gone_id = gone.id();
    c.add_peer_addr(gone.addr());
    gone.shutdown().await.unwrap();
    let bytes = data(12 * 1024 * 1024, 2);
    let hash = a
        .blobs()
        .add_bytes(bytes.clone(), Protection::Library)
        .await
        .unwrap();
    c.add_peer_addr(a.addr());
    c.add_peer_addr(empty.addr());

    let started = Instant::now();
    let report = timeout(
        WAIT,
        c.download_blob_from(
            &[empty.id(), gone_id, a.id()],
            &hash,
            Some(bytes.len() as u64),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        c.blobs().get(&hash).await.unwrap().as_ref(),
        bytes.as_slice()
    );
    assert_eq!(report.main(), Some(a.id()));
    assert!(
        report.sources.iter().all(|(p, _)| *p == a.id()),
        "only the holder that has it sent anything: {report:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(9),
        "did not wait out the offline holder's connect timeout: {:?}",
        started.elapsed()
    );
    for n in [&a, &empty, &c] {
        n.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_small_blob_comes_from_the_next_holder_when_the_first_has_not_got_it() {
    let dir = tempfile::tempdir().unwrap();
    let (a, empty, c) = (
        open(&dir, "a").await,
        open(&dir, "empty").await,
        open(&dir, "c").await,
    );
    let bytes = data(300_000, 3);
    let hash = a
        .blobs()
        .add_bytes(bytes.clone(), Protection::Library)
        .await
        .unwrap();
    c.add_peer_addr(a.addr());
    c.add_peer_addr(empty.addr());

    let report = timeout(
        WAIT,
        c.fetch_blob_from(
            &[empty.id(), a.id()],
            &hash,
            Some(bytes.len() as u64),
            Protection::Library,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        c.blobs().get(&hash).await.unwrap().as_ref(),
        bytes.as_slice()
    );
    assert_eq!(report.main(), Some(a.id()));

    // Nobody has it: a clean error naming the holders tried.
    let nothing = shoebox_core::ids::Blake3Hash::of_bytes(b"nobody has this");
    let err = timeout(
        WAIT,
        c.download_blob_from(&[empty.id(), a.id()], &nothing, Some(9 * 1024 * 1024)),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert!(err.to_string().contains("2 holders"), "{err:#}");
    for n in [&a, &empty, &c] {
        n.shutdown().await.unwrap();
    }
}

/// The Devices screen's "right now": a download shows on the fetching node while it runs, the
/// holder shows what it sends, and both report a speed afterwards.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_fetch_shows_as_a_download_here_and_an_upload_on_the_holder() {
    let dir = tempfile::tempdir().unwrap();
    let (a, c) = (open(&dir, "a").await, open(&dir, "c").await);
    let bytes = data(16 * 1024 * 1024, 3);
    let hash = a
        .blobs()
        .add_bytes(bytes.clone(), Protection::Library)
        .await
        .unwrap();
    c.add_peer_addr(a.addr());

    let fetch = {
        let (c, from) = (c.clone(), a.id());
        let len = bytes.len() as u64;
        tokio::spawn(async move {
            c.fetch_blob_from(&[from], &hash, Some(len), Protection::Library)
                .await
        })
    };
    let (mut seen_down, mut seen_up) = (false, false);
    let a_core = shoebox_node::from_iroh_id(&a.id());
    let c_core = shoebox_node::from_iroh_id(&c.id());
    let end = Instant::now() + WAIT;
    while !fetch.is_finished() && Instant::now() < end {
        let down = c.transfers();
        if let Some(t) = down.downloads.iter().find(|t| t.hash == hash) {
            assert_eq!(t.size, Some(bytes.len() as u64));
            assert_eq!(t.peer, Some(a_core));
            seen_down = true;
        }
        if a.transfers()
            .uploads
            .iter()
            .any(|t| t.hash == hash && t.peer == Some(c_core))
        {
            seen_up = true;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    fetch.await.unwrap().unwrap();
    assert!(seen_down, "the download was listed while it ran");
    assert!(
        seen_up,
        "the holder listed what it was sending, and to whom"
    );
    assert!(
        c.transfers().downloads.is_empty(),
        "and nothing once it is done"
    );
    assert!(c.transfers().download_rate > 0);
    assert!(a.transfers().upload_rate > 0);
    for n in [&a, &c] {
        n.shutdown().await.unwrap();
    }
}
