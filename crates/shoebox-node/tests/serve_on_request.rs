//! Spike: a holder keeps no copy of a blob, only its hash. When a GET for that hash arrives, the
//! provider's intercept event holds the request while the holder exports the bytes to a temp
//! file (standing in for PhotoKit), imports it by reference, and only then lets the request go
//! on. Once the transfer ends the holder drops its tag and the temp file, and GC forgets it.
//!
//! The requester is an unchanged Shoebox node using its normal download path.

use iroh::endpoint::presets;
use iroh::protocol::Router;
use iroh::Endpoint;
use iroh_blobs::api::blobs::AddPathOptions;
use iroh_blobs::api::proto::ImportMode;
use iroh_blobs::provider::events::{
    EventMask, EventSender, ProviderMessage, RequestMode, RequestUpdate,
};
use iroh_blobs::store::fs::{options::Options, FsStore};
use iroh_blobs::store::GcConfig;
use iroh_blobs::{BlobFormat, BlobsProtocol, Hash, ALPN};
use shoebox_core::ids::Blake3Hash;
use shoebox_node::{Node, NodeConfig};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::timeout;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_blob_imported_only_when_asked_for_reaches_the_peer() {
    let dir = tempfile::tempdir().unwrap();
    // The "photo library": bytes the holder can produce on demand, keyed by their hash.
    let photo: Vec<u8> = (0..3_000_000u32)
        .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
        .collect();
    let hash = Hash::new(&photo);
    let library: Arc<HashMap<Hash, Vec<u8>>> = Arc::new([(hash, photo.clone())].into());

    // Holder: a bare iroh-blobs store (GC on) with GET requests intercepted.
    let mut opts = Options::new(&dir.path().join("holder"));
    opts.gc = Some(GcConfig {
        interval: Duration::from_millis(200),
        add_protected: None,
    });
    let store = FsStore::load_with_opts(dir.path().join("holder/blobs.db"), opts)
        .await
        .unwrap();
    let mask = EventMask {
        get: RequestMode::InterceptLog,
        ..EventMask::DEFAULT
    };
    let (events, mut rx) = EventSender::channel(64, mask);
    let served: Arc<Mutex<Vec<&'static str>>> = Arc::default();
    {
        let (store, library, served) = (store.clone(), library.clone(), served.clone());
        let tmp = dir.path().join("export");
        std::fs::create_dir_all(&tmp).unwrap();
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                let ProviderMessage::GetRequestReceived(m) = msg else {
                    continue;
                };
                let (store, library, served, tmp) =
                    (store.clone(), library.clone(), served.clone(), tmp.clone());
                // One task per request: the event loop must not wait on an export.
                tokio::spawn(async move {
                    let want = m.inner.request.hash;
                    let had = store.blobs().has(want).await.unwrap();
                    served
                        .lock()
                        .unwrap()
                        .push(if had { "already-held" } else { "prepared" });
                    let Some(bytes) = library.get(&want) else {
                        // Not ours: let it through, the store answers with nothing.
                        let _ = m.tx.send(Ok(())).await;
                        return;
                    };
                    // "PhotoKit export" (slow on purpose), then import by reference.
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    let path: PathBuf = tmp.join(want.to_hex());
                    tokio::fs::write(&path, bytes).await.unwrap();
                    let tag = store
                        .blobs()
                        .add_path_with_opts(AddPathOptions {
                            path: path.clone(),
                            format: BlobFormat::Raw,
                            mode: ImportMode::TryReference,
                        })
                        .temp_tag()
                        .await
                        .unwrap();
                    assert_eq!(tag.hash(), want);
                    m.tx.send(Ok(())).await.unwrap();
                    // Hold the tag until the transfer ends, then let go of everything.
                    let mut updates = m.rx;
                    while let Ok(Some(u)) = updates.recv().await {
                        if matches!(u, RequestUpdate::Completed(_) | RequestUpdate::Aborted(_)) {
                            break;
                        }
                    }
                    drop(tag);
                    let _ = tokio::fs::remove_file(&path).await;
                    served.lock().unwrap().push("released");
                });
            }
        });
    }
    let holder_ep = Endpoint::builder(presets::Minimal).bind().await.unwrap();
    let _router = Router::builder(holder_ep.clone())
        .accept(ALPN, BlobsProtocol::new(&store, Some(events)))
        .spawn();
    assert!(
        !store.blobs().has(hash).await.unwrap(),
        "holder keeps no copy"
    );

    // Requester: an ordinary Shoebox node, normal download path.
    let requester = Node::open(NodeConfig::local_only(dir.path().join("req"), "req"))
        .await
        .unwrap();
    requester.add_peer_addr(holder_ep.addr());
    let b3 = Blake3Hash(*hash.as_bytes());
    timeout(
        Duration::from_secs(60),
        requester.download_blob_from(&[holder_ep.id()], &b3, None),
    )
    .await
    .expect("download in time")
    .expect("download succeeds");
    assert_eq!(
        requester.blobs().get(&b3).await.unwrap().as_ref(),
        photo.as_slice()
    );

    // The holder released its temp copy, and GC forgets the entry.
    timeout(Duration::from_secs(20), async {
        while !served.lock().unwrap().contains(&"released")
            || store.blobs().has(hash).await.unwrap()
        {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("holder lets go after serving");
    assert_eq!(served.lock().unwrap().first(), Some(&"prepared"));
    assert!(std::fs::read_dir(dir.path().join("export"))
        .unwrap()
        .next()
        .is_none());
}
