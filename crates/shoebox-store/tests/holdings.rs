//! M1-11: holdings and copy counts across three nodes, including removals and an offline node.

mod common;

use common::*;
use shoebox_core::ids::Blake3Hash;
use shoebox_core::model::BlobKind;
use shoebox_core::store::*;
use shoebox_node::{ImportOptions, LibraryNode};
use shoebox_store::Library;
use shoebox_testkit::{wait_until, Cluster};
use std::path::{Path, PathBuf};
use std::time::Duration;

async fn counts(nodes: &[LibraryNode<Library>], id: &Blake3Hash) -> Vec<usize> {
    let mut out = vec![];
    for n in nodes {
        out.push(n.copy_count(id).await.unwrap());
    }
    out
}

async fn settle_to(nodes: &[LibraryNode<Library>], id: &Blake3Hash, want: usize, what: &str) {
    wait_until(Duration::from_secs(30), || async {
        counts(nodes, id).await.iter().all(|c| *c == want)
    })
    .await
    .unwrap_or_else(|_| panic!("{what}: copy count never reached {want} on every node"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn copy_counts_follow_adds_removals_and_an_offline_node() {
    let c = Cluster::new(3).await.unwrap();
    let nodes = library_nodes_auto(&c).await;
    let samples: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples");
    let files = [
        samples.join("photo-gps.jpg"),
        samples.join("IMG_0001.jpg"),
        samples.join("screenshot.png"),
    ];
    let report = nodes[0]
        .import_paths(&files, ImportOptions::default())
        .await
        .unwrap();
    assert_eq!(report.imported.len(), 3, "{report:?}");
    let (a, b, cc) = (report.imported[0], report.imported[1], report.imported[2]);

    // Everyone learns about the 3 photos; only node 0 holds the originals.
    settle_to(&nodes, &a, 1, "node 0 holds it").await;
    for n in &nodes {
        assert_eq!(n.redundancy_target().await.unwrap(), 2, "default target");
        let mut risky = n.at_risk(Page::all()).await.unwrap();
        risky.sort();
        let mut all = vec![a, b, cc];
        all.sort();
        assert_eq!(risky, all, "a single copy is below the target of 2");
    }

    // Node 1 now holds `a` too (as if it had fetched it).
    nodes[1]
        .publish_holdings(vec![(a, BlobKind::Original)])
        .await
        .unwrap();
    settle_to(&nodes, &a, 2, "after node 1 adds a copy").await;
    for n in &nodes {
        assert!(
            !n.at_risk(Page::all()).await.unwrap().contains(&a),
            "a has reached the target"
        );
        assert!(n.at_risk(Page::all()).await.unwrap().contains(&b));
    }
    assert_eq!(nodes[2].holders(&a).await.unwrap().len(), 2);

    // Node 2 loses the network and adds a copy: nobody else can see it yet.
    c.take_offline(2);
    nodes[2]
        .publish_holdings(vec![(a, BlobKind::Original)])
        .await
        .unwrap();
    assert_eq!(
        nodes[2].copy_count(&a).await.unwrap(),
        3,
        "it knows its own copy"
    );
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(nodes[0].copy_count(&a).await.unwrap(), 2);
    assert_eq!(nodes[1].copy_count(&a).await.unwrap(), 2);
    c.bring_online(2);
    settle_to(&nodes, &a, 3, "after the offline node reconnects").await;

    // Node 1 drops its copy; the count falls everywhere. A thumbnail never counts as a copy.
    nodes[1].retract_holdings(vec![a]).await.unwrap();
    nodes[1]
        .publish_holdings(vec![(b, BlobKind::Thumb)])
        .await
        .unwrap();
    settle_to(&nodes, &a, 2, "after node 1 removes its copy").await;
    settle_to(&nodes, &b, 1, "a thumb is not a copy").await;

    // The redundancy target is a replicated library setting.
    nodes[0].set_redundancy_target(3).await.unwrap();
    wait_until(Duration::from_secs(20), || async {
        let mut ok = true;
        for n in &nodes {
            ok &= n.redundancy_target().await.unwrap() == 3;
        }
        ok
    })
    .await
    .expect("the target reaches every device");
    assert!(
        nodes[2].at_risk(Page::all()).await.unwrap().contains(&a),
        "2 copies < target 3"
    );
    assert!(
        nodes[0].set_redundancy_target(0).await.is_err(),
        "0 is not a sensible target"
    );

    // Offline removal: node 2 retracts while cut off; the others keep counting it until it is back.
    c.take_offline(2);
    nodes[2].retract_holdings(vec![a]).await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(nodes[0].copy_count(&a).await.unwrap(), 2);
    c.bring_online(2);
    settle_to(&nodes, &a, 1, "after the removal propagates").await;
    c.shutdown().await;
}
