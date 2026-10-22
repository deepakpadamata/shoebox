use iroh_docs::api::protocol::{AddrInfoOptions, ShareMode};
use shoebox_node::{doc_digest, Node, NodeConfig};

#[tokio::test]
async fn digest_is_order_independent_changes_with_content_and_is_served_to_peers() {
    let dir = tempfile::tempdir().unwrap();
    let a = Node::open(NodeConfig::local_only(dir.path().join("a"), "a"))
        .await
        .unwrap();
    let b = Node::open(NodeConfig::local_only(dir.path().join("b"), "b"))
        .await
        .unwrap();
    b.add_peer_addr(a.addr());
    let author = a.docs().author_default().await.unwrap();

    let doc = a.docs().create().await.unwrap();
    doc.start_sync(vec![]).await.unwrap();
    let empty = doc_digest(&doc).await.unwrap();
    doc.set_bytes(author, "k/1", "one").await.unwrap();
    let one = doc_digest(&doc).await.unwrap();
    assert_ne!(empty, one);
    doc.set_bytes(author, "k/2", "two").await.unwrap();
    let two = doc_digest(&doc).await.unwrap();
    assert_ne!(one, two);
    assert_eq!(
        two,
        doc_digest(&doc).await.unwrap(),
        "stable for unchanged content"
    );
    doc.set_bytes(author, "k/2", "TWO").await.unwrap();
    assert_ne!(
        two,
        doc_digest(&doc).await.unwrap(),
        "an overwritten value changes the digest"
    );

    // A peer can ask for it, and gets exactly the local digest.
    let local = doc_digest(&doc).await.unwrap();
    let remote = b.peer_doc_digest(a.id(), doc.id()).await.unwrap();
    assert_eq!(remote, Some(local));

    // A namespace the peer does not have is reported as such rather than guessed at.
    let other = b.docs().create().await.unwrap();
    assert_eq!(b.peer_doc_digest(a.id(), other.id()).await.unwrap(), None);

    // Same entries synced to another node give the same digest on both.
    let ticket = doc
        .share(ShareMode::Write, AddrInfoOptions::Addresses)
        .await
        .unwrap();
    let (doc_b, _events) = b.docs().import_and_subscribe(ticket).await.unwrap();
    let mut equal = false;
    for _ in 0..100 {
        if doc_digest(&doc_b).await.unwrap() == local {
            equal = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(equal, "replicas with the same entries have the same digest");
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
}
