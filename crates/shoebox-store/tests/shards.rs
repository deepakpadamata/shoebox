//! S1-07: every device holds every live index shard and lets go of replaced and bad ones.

mod common;

use common::*;
use shoebox_core::ids::Blake3Hash;
use shoebox_core::model::{BlobKind, BlobRef, IndexShard, ModelId, PhotoId, Role, ShardKind};
use shoebox_core::store::LibraryStore;
use shoebox_node::{FetcherConfig, LibraryNode, Protection, SHARD_MIME};
use shoebox_search::shard::{Rows, Shard, Vec8};
use shoebox_store::Library;
use shoebox_testkit::{wait_until, Cluster};
use std::time::Duration;

fn fast_fetch() -> FetcherConfig {
    FetcherConfig {
        concurrency: 4,
        tick: Duration::from_millis(300),
        retry: Duration::from_millis(300),
    }
}

const MODEL: ModelId = ModelId([3; 32]);

fn pid(i: u32) -> PhotoId {
    Blake3Hash::of_bytes(&i.to_le_bytes())
}

fn shard(photos: std::ops::Range<u32>, replaces: Vec<Blake3Hash>) -> Shard {
    let ids: Vec<PhotoId> = photos.map(pid).collect();
    let rows = ids
        .iter()
        .map(|p| {
            Vec8::quantize(
                &p.0[..16]
                    .iter()
                    .map(|b| f32::from(*b) - 127.5)
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    Shard::build(MODEL, 16, replaces, Rows::Vec8(rows), ids).unwrap()
}

async fn kept(n: &LibraryNode<Library>, h: &Blake3Hash) -> bool {
    n.node().blobs().kept(h).await.unwrap()
}

async fn holds_all(n: &LibraryNode<Library>, hashes: &[Blake3Hash]) -> bool {
    for h in hashes {
        if !kept(n, h).await {
            return false;
        }
    }
    true
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shards_reach_every_device_and_replaced_ones_are_released() {
    let c = Cluster::with_roles(&[Role::Keeper, Role::Companion, Role::Viewer])
        .await
        .unwrap();
    let nodes = library_nodes_auto(&c).await;
    nodes[1].start_fetcher(fast_fetch());
    nodes[2].start_fetcher(fast_fetch());

    let mut first = vec![];
    for range in [0..10, 10..20, 20..30] {
        first.push(
            nodes[0]
                .publish_shard(&shard(range, vec![]))
                .await
                .unwrap()
                .blob
                .hash,
        );
    }
    for n in &nodes[1..] {
        wait_until(Duration::from_secs(60), || async {
            holds_all(n, &first).await
        })
        .await
        .expect("every device holds every shard");
        let view = n.shard_view().await.unwrap();
        assert_eq!(view.live.len(), 3);
    }

    // Compaction on A: one shard replaces the first two, leaving photo 0 out.
    let merged = nodes[0]
        .publish_shard(&shard(1..20, vec![first[0], first[1]]))
        .await
        .unwrap()
        .blob
        .hash;
    for n in &nodes {
        wait_until(Duration::from_secs(60), || async { kept(n, &merged).await })
            .await
            .expect("the new shard arrives");
        n.reconcile_blobs().await.unwrap();
        assert!(
            !kept(n, &first[0]).await && !kept(n, &first[1]).await,
            "replaced shards released"
        );
        assert!(kept(n, &first[2]).await);
        let view = n.shard_view().await.unwrap();
        assert_eq!(view.live, [merged, first[2]].into_iter().collect());
    }
    // Their holdings are retracted, so nobody asks for them.
    wait_until(Duration::from_secs(30), || async {
        nodes[2]
            .store()
            .holders(&first[0])
            .await
            .unwrap()
            .is_empty()
    })
    .await
    .expect("holdings of the replaced shards are withdrawn");

    // Freeing up space never touches search data.
    nodes[1].free_up_space(false).await.unwrap();
    assert!(holds_all(&nodes[1], &[merged, first[2]]).await);
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_shard_that_does_not_match_its_record_is_ignored() {
    let c = Cluster::with_roles(&[Role::Keeper, Role::Companion])
        .await
        .unwrap();
    let nodes = library_nodes_auto(&c).await;
    nodes[1].start_fetcher(fast_fetch());

    // A record that claims a text shard; the blob is a vec8 shard.
    let bytes = shard(0..4, vec![]).encode();
    let hash = nodes[0]
        .node()
        .blobs()
        .add_bytes(bytes.clone(), Protection::Index)
        .await
        .unwrap();
    nodes[0]
        .store()
        .put_index_shard(IndexShard {
            blob: BlobRef {
                hash,
                size: bytes.len() as u64,
                mime: SHARD_MIME.into(),
            },
            model: MODEL,
            kind: ShardKind::Text,
            count: 4,
            replaces: vec![],
            made_by: nodes[0].device(),
            hlc: shoebox_core::hlc::Hlc::GENESIS,
        })
        .await
        .unwrap();
    nodes[0]
        .publish_holdings(vec![(hash, BlobKind::Index)])
        .await
        .unwrap();

    wait_until(Duration::from_secs(60), || async {
        nodes[1].shard_view().await.unwrap().bad.contains(&hash)
    })
    .await
    .expect("fetched and found bad");
    let view = nodes[1].shard_view().await.unwrap();
    assert!(view.live.is_empty() && !view.wanted.contains(&hash));
    assert_eq!(
        nodes[1].store().index_shards().await.unwrap().len(),
        1,
        "the record stays"
    );
    nodes[1].reconcile_blobs().await.unwrap();
    assert!(!kept(&nodes[1], &hash).await, "and the blob is let go");
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_phone_gets_a_model_file_from_a_desktop_and_a_wrong_file_is_refused() {
    use shoebox_search::models::ModelFile;
    let c = Cluster::with_roles(&[Role::Keeper, Role::Companion])
        .await
        .unwrap();
    let nodes = library_nodes_auto(&c).await;
    let dir = tempfile::tempdir().unwrap();
    let bytes: Vec<u8> = (0..300_000u32).map(|i| (i * 7 % 251) as u8).collect();
    let file = ModelFile {
        name: "test-model.onnx",
        blake3: *blake3::hash(&bytes).as_bytes(),
        size: bytes.len() as u64,
    };
    let path = dir.path().join(file.name);
    std::fs::write(&path, &bytes).unwrap();

    assert_eq!(
        nodes[1].model_bytes(&file).await.unwrap(),
        None,
        "nobody offers it yet"
    );
    nodes[0].share_model(&file, &path).await.unwrap();
    wait_until(Duration::from_secs(30), || async {
        nodes[1].model_offered(&file).await.unwrap()
    })
    .await
    .expect("the offer reaches the phone");
    let got = nodes[1].model_bytes(&file).await.unwrap().expect("fetched");
    assert_eq!(&got[..], &bytes[..]);
    assert!(nodes[1]
        .node()
        .blobs()
        .protections(&Blake3Hash(file.blake3))
        .await
        .unwrap()
        .contains(&Protection::Index));

    // A local file that is not the model is not offered.
    let wrong = dir.path().join("wrong.onnx");
    std::fs::write(&wrong, b"not the model").unwrap();
    let other = ModelFile {
        name: "other.onnx",
        blake3: [9; 32],
        size: 13,
    };
    assert!(nodes[0].share_model(&other, &wrong).await.is_err());
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_old_models_shards_go_once_the_new_model_covers_as_much() {
    let c = Cluster::with_roles(&[Role::Keeper]).await.unwrap();
    let nodes = library_nodes_auto(&c).await;
    let n = &nodes[0];
    let new = ModelId([4; 32]);
    n.set_search_model(new);
    let old_shard = n
        .publish_shard(&shard(0..10, vec![]))
        .await
        .unwrap()
        .blob
        .hash;
    let mut partial = shard(0..5, vec![]);
    partial.model = new;
    n.publish_shard(&partial).await.unwrap();
    n.reconcile_blobs().await.unwrap();
    assert!(
        kept(n, &old_shard).await,
        "the new model covers fewer photos: keep the old"
    );
    let mut rest = shard(5..10, vec![]);
    rest.model = new;
    n.publish_shard(&rest).await.unwrap();
    n.reconcile_blobs().await.unwrap();
    assert!(
        !kept(n, &old_shard).await,
        "covered as well: the old model's shards go"
    );
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_record_whose_blob_never_arrives_changes_nothing() {
    let c = Cluster::with_roles(&[Role::Keeper]).await.unwrap();
    let nodes = library_nodes_auto(&c).await;
    let n = &nodes[0];
    let good = n
        .publish_shard(&shard(0..4, vec![]))
        .await
        .unwrap()
        .blob
        .hash;
    // A record (claiming to replace the good shard) for a blob nobody holds.
    let missing = shard(0..4, vec![good]).encode();
    n.store()
        .put_index_shard(IndexShard {
            blob: BlobRef {
                hash: Blake3Hash::of_bytes(&missing),
                size: missing.len() as u64,
                mime: SHARD_MIME.into(),
            },
            model: MODEL,
            kind: ShardKind::Vec8,
            count: 4,
            replaces: vec![good],
            made_by: n.device(),
            hlc: shoebox_core::hlc::Hlc::GENESIS,
        })
        .await
        .unwrap();
    let view = n.shard_view().await.unwrap();
    assert_eq!(
        view.live,
        [good].into_iter().collect(),
        "the good shard still counts"
    );
    n.reconcile_blobs().await.unwrap();
    assert!(
        kept(n, &good).await,
        "and is kept until its replacement really arrives"
    );
    c.shutdown().await;
}
