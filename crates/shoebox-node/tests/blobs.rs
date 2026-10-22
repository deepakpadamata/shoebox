//! M1-03: blob store integration — add/get/range/has, protection tags vs GC, import by reference,
//! verified peer transfer.

use shoebox_core::ids::Blake3Hash;
use shoebox_core::model::AlbumId;
use shoebox_node::{ImportMode, Node, NodeConfig, NodeEvent, Protection};
use std::time::Duration;
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(20);

async fn open(dir: &tempfile::TempDir, name: &str) -> Node {
    let cfg = NodeConfig::local_only(dir.path().join(name), name)
        .with_gc_interval(Duration::from_millis(100))
        // GC steps aside while the node is busy; these tests add blobs and then wait for GC.
        .with_gc_quiet(Duration::from_millis(150));
    Node::open(cfg).await.expect("node opens")
}

/// Poll until `blob` is gone (GC runs on a 100 ms interval in these tests).
async fn gone(node: &Node, hash: &Blake3Hash) -> bool {
    for _ in 0..200 {
        if !node.blobs().has(hash).await.unwrap() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

#[tokio::test]
async fn add_get_has_size_and_ranges() {
    let dir = tempfile::tempdir().unwrap();
    let node = open(&dir, "a").await;
    let blobs = node.blobs();
    let data: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
    let hash = blobs
        .add_bytes(data.clone(), Protection::Library)
        .await
        .unwrap();
    assert_eq!(
        hash,
        Blake3Hash::of_bytes(&data),
        "photo ids are the BLAKE3 of the bytes"
    );
    assert!(blobs.has(&hash).await.unwrap());
    assert_eq!(blobs.size(&hash).await.unwrap(), Some(100_000));
    assert_eq!(blobs.get(&hash).await.unwrap().as_ref(), data.as_slice());
    assert_eq!(
        blobs.open_range(&hash, 10..20).await.unwrap().as_ref(),
        &data[10..20]
    );
    assert_eq!(
        blobs
            .open_range(&hash, 99_990..200_000)
            .await
            .unwrap()
            .as_ref(),
        &data[99_990..]
    );
    assert!(blobs.open_range(&hash, 5..5).await.unwrap().is_empty());

    let missing = Blake3Hash::of_bytes(b"never added");
    assert!(!blobs.has(&missing).await.unwrap());
    assert_eq!(blobs.size(&missing).await.unwrap(), None);
    assert!(blobs.get(&missing).await.is_err());
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn add_path_copy_and_reference_modes() {
    let dir = tempfile::tempdir().unwrap();
    let node = open(&dir, "a").await;
    let file = dir.path().join("photo.bin");
    // 48 MB of non-trivial data (sparse files would hash as zeros, which is fine but boring).
    let data: Vec<u8> = (0..48 * 1024 * 1024u32)
        .map(|i| (i.wrapping_mul(2654435761) >> 24) as u8)
        .collect();
    std::fs::write(&file, &data).unwrap();
    let blobs = node.blobs();

    let by_ref = blobs
        .add_path(&file, ImportMode::Reference, Protection::Library)
        .await
        .unwrap();
    assert_eq!(by_ref, Blake3Hash::of_bytes(&data));
    let stored = dir_size(&dir.path().join("a/blobs"));
    assert!(
        stored < 8 * 1024 * 1024,
        "referenced import must not copy the file: {stored} bytes stored"
    );
    assert_eq!(
        blobs
            .open_range(&by_ref, 1000..1010)
            .await
            .unwrap()
            .as_ref(),
        &data[1000..1010]
    );

    // Copy mode of different content does store the bytes.
    let file2 = dir.path().join("photo2.bin");
    let data2: Vec<u8> = data.iter().rev().take(10 * 1024 * 1024).copied().collect();
    std::fs::write(&file2, &data2).unwrap();
    let copied = blobs
        .add_path(&file2, ImportMode::Copy, Protection::Library)
        .await
        .unwrap();
    assert_eq!(copied, Blake3Hash::of_bytes(&data2));
    assert!(dir_size(&dir.path().join("a/blobs")) >= stored + 10 * 1024 * 1024);

    assert!(blobs
        .add_path(
            &dir.path().join("nope"),
            ImportMode::Copy,
            Protection::Library
        )
        .await
        .is_err());
    node.shutdown().await.unwrap();
}

fn dir_size(path: &std::path::Path) -> u64 {
    std::fs::read_dir(path)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            let meta = e.metadata().unwrap();
            if meta.is_dir() {
                dir_size(&e.path())
            } else {
                meta.len()
            }
        })
        .sum()
}

#[tokio::test]
async fn gc_removes_only_unprotected_blobs() {
    let dir = tempfile::tempdir().unwrap();
    let node = open(&dir, "a").await;
    let blobs = node.blobs();
    let keep = blobs
        .add_bytes(b"library blob".to_vec(), Protection::Library)
        .await
        .unwrap();
    let pinned = blobs
        .add_bytes(b"pinned blob".to_vec(), Protection::Pin)
        .await
        .unwrap();
    let doomed = blobs
        .add_bytes(b"doomed blob".to_vec(), Protection::Library)
        .await
        .unwrap();
    let two = blobs
        .add_bytes(b"two protections".to_vec(), Protection::Library)
        .await
        .unwrap();
    blobs.protect(&two, Protection::Pin).await.unwrap();

    assert_eq!(
        blobs.protections(&two).await.unwrap(),
        vec![Protection::Library, Protection::Pin]
    );
    blobs.remove(&doomed).await.unwrap();
    assert!(
        gone(&node, &doomed).await,
        "an unprotected blob is collected"
    );

    // Dropping one of two protections keeps the blob; dropping the last one frees it.
    blobs.unprotect(&two, Protection::Library).await.unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(blobs.has(&two).await.unwrap());
    blobs.unprotect(&two, Protection::Pin).await.unwrap();
    assert!(gone(&node, &two).await);

    assert!(
        blobs.has(&keep).await.unwrap(),
        "library-referenced blob survives GC"
    );
    assert!(blobs.has(&pinned).await.unwrap(), "pinned blob survives GC");
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn album_references_are_protections_too() {
    let dir = tempfile::tempdir().unwrap();
    let node = open(&dir, "a").await;
    let blobs = node.blobs();
    let album = Protection::Album(AlbumId([7; 16]));
    let h = blobs
        .add_bytes(b"shared album preview".to_vec(), Protection::Library)
        .await
        .unwrap();
    blobs.protect(&h, album.clone()).await.unwrap();
    blobs.remove(&h).await.unwrap(); // the library lets go of everything it owns...
                                     // ...but `remove` only drops library-owned protections; an album reference still holds it.
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(blobs.has(&h).await.unwrap());
    assert_eq!(blobs.protections(&h).await.unwrap(), vec![album.clone()]);
    blobs.unprotect(&h, album).await.unwrap();
    assert!(gone(&node, &h).await);
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn protections_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let node = open(&dir, "a").await;
    let h = node
        .blobs()
        .add_bytes(b"durable".to_vec(), Protection::Pin)
        .await
        .unwrap();
    node.shutdown().await.unwrap();
    let node = open(&dir, "a").await;
    tokio::time::sleep(Duration::from_millis(600)).await; // several GC cycles
    assert!(node.blobs().has(&h).await.unwrap());
    assert_eq!(
        node.blobs().protections(&h).await.unwrap(),
        vec![Protection::Pin]
    );
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn blob_fetched_from_a_peer_is_stored_protected_and_reported() {
    let dir = tempfile::tempdir().unwrap();
    let a = open(&dir, "a").await;
    let b = open(&dir, "b").await;
    b.add_peer_addr(a.addr());
    let data: Vec<u8> = (0..3_000_000u32).map(|i| (i % 253) as u8).collect();
    let hash = a
        .blobs()
        .add_bytes(data.clone(), Protection::Library)
        .await
        .unwrap();

    let mut events = b.events();
    timeout(WAIT, b.fetch_blob(a.id(), &hash, Protection::Library))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        b.blobs().get(&hash).await.unwrap().as_ref(),
        data.as_slice()
    );
    assert_eq!(
        b.blobs().protections(&hash).await.unwrap(),
        vec![Protection::Library]
    );

    let mut saw_progress = false;
    let mut saw_done = false;
    while let Ok(res) = timeout(Duration::from_millis(500), events.recv()).await {
        let Ok(ev) = res else { continue }; // lagged: keep reading
        match ev {
            NodeEvent::BlobProgress {
                hash: h,
                from,
                bytes,
            } if h == hash => {
                assert_eq!(from, a.id());
                assert!(bytes > 0);
                saw_progress = true;
            }
            NodeEvent::BlobFetched { hash: h, from } if h == hash => {
                assert_eq!(from, a.id());
                saw_done = true;
            }
            _ => {}
        }
    }
    assert!(
        saw_progress && saw_done,
        "progress={saw_progress} done={saw_done}"
    );

    // Fetching something the peer does not have fails cleanly.
    let nothing = Blake3Hash::of_bytes(b"nobody has this");
    let res = timeout(WAIT, b.fetch_blob(a.id(), &nothing, Protection::Library))
        .await
        .unwrap();
    assert!(res.is_err());
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
}

#[tokio::test]
async fn corrupted_transfer_data_is_rejected() {
    use iroh_blobs::protocol::ChunkRanges;
    let dir = tempfile::tempdir().unwrap();
    let a = open(&dir, "a").await;
    let b = open(&dir, "b").await;
    let data: Vec<u8> = (0..200_000u32).map(|i| (i % 249) as u8).collect();
    let hash = a
        .blobs()
        .add_bytes(data, Protection::Library)
        .await
        .unwrap();
    let iroh_hash = iroh_blobs::Hash::from_bytes(hash.0);

    // The wire encoding of the blob (BAO), as a peer would send it.
    let mut bao = a
        .blobs()
        .raw()
        .blobs()
        .export_bao(iroh_hash, ChunkRanges::all())
        .bao_to_vec()
        .await
        .unwrap();
    let good = bao.clone();
    let last = bao.len() - 1;
    bao[last] ^= 0xFF; // flip a bit in the final data chunk

    let store = b.blobs().raw().blobs();
    assert!(
        store
            .import_bao_bytes(iroh_hash, ChunkRanges::all(), bao)
            .await
            .is_err(),
        "corruption is detected"
    );
    assert!(!b.blobs().has(&hash).await.unwrap(), "nothing was stored");
    // The untampered stream is accepted.
    store
        .import_bao_bytes(iroh_hash, ChunkRanges::all(), good)
        .await
        .unwrap();
    assert!(b.blobs().has(&hash).await.unwrap());
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
}

/// Acceptance check for M1-03 at full size. Slow (writes and hashes 2 GB), so opt-in:
/// `cargo test -p shoebox-node --test blobs -- --ignored two_gigabyte`
#[tokio::test]
#[ignore = "slow: 2 GB"]
async fn two_gigabyte_file_imports_by_reference_without_copying() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let node = open(&dir, "a").await;
    let file = dir.path().join("big.bin");
    {
        let mut f = std::io::BufWriter::new(std::fs::File::create(&file).unwrap());
        let chunk: Vec<u8> = (0..1024 * 1024u32)
            .map(|i| (i.wrapping_mul(2654435761) >> 24) as u8)
            .collect();
        for i in 0..2048u32 {
            let mut c = chunk.clone();
            c[0] = i as u8;
            c[1] = (i >> 8) as u8;
            f.write_all(&c).unwrap();
        }
    }
    let hash = node
        .blobs()
        .add_path(&file, ImportMode::Reference, Protection::Library)
        .await
        .unwrap();
    assert_eq!(
        node.blobs().size(&hash).await.unwrap(),
        Some(2048 * 1024 * 1024)
    );
    let stored = dir_size(&dir.path().join("a/blobs"));
    assert!(
        stored < 64 * 1024 * 1024,
        "2 GB file must not be copied: {stored} bytes stored"
    );
    node.shutdown().await.unwrap();
}
