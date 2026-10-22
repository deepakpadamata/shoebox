//! M5: shared albums between separate households on real nodes: create, invite, join, add,
//! fetch, roles, expiry, and the connection gate.

use shoebox_core::identity::KeyPair;
use shoebox_core::ids::Blake3Hash;
use shoebox_core::model::*;
use shoebox_core::store::*;
use shoebox_core::Timestamp;
use shoebox_node::{LibraryNode, Node, Protection};
use shoebox_store::{decode_invite, encode_invite, HybridBackend, SharedAlbums};
use shoebox_testkit::{wait_until, Cluster};
use std::sync::Arc;
use std::time::Duration;

/// Give a node its own library (so its connection gate is armed, like a real household).
async fn household(c: &Cluster, i: usize) -> LibraryNode<shoebox_store::Library> {
    let root = KeyPair::generate();
    let backend = HybridBackend::for_node(c.node(i).clone(), c.clock().clone());
    let (lib, cap) = backend.create_library(&root.root_public()).await.unwrap();
    let ln =
        LibraryNode::new(c.node(i).clone(), Arc::new(lib), c.clock().clone()).with_capability(cap);
    ln.create_identity(&root, Role::Keeper, "home")
        .await
        .unwrap();
    ln.enforce_membership().await.unwrap();
    ln
}

async fn blob(node: &Node, seed: u8, len: usize) -> BlobRef {
    let data = vec![seed; len];
    let hash = node
        .blobs()
        .add_bytes(data, Protection::Library)
        .await
        .unwrap();
    BlobRef {
        hash,
        size: len as u64,
        mime: "image/jpeg".into(),
    }
}

async fn item(node: &Node, seed: u8) -> SharedItem {
    let original = blob(node, seed, 5_000).await;
    let preview = blob(node, seed.wrapping_add(50), 800).await;
    let thumb = blob(node, seed.wrapping_add(100), 100).await;
    SharedItem {
        id: original.hash,
        original,
        shared_hash: None,
        preview,
        thumb,
        added_by: shoebox_node::from_iroh_id(&node.id()),
        added_at: Timestamp(seed as i64),
        taken_at: Timestamp(0),
    }
}

async fn open(c: &Cluster, i: usize, dir: &std::path::Path, bg: bool) -> SharedAlbums {
    SharedAlbums::open(c.node(i).clone(), dir, bg)
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_households_share_an_album_and_roles_are_enforced() {
    let c = Cluster::new(3).await.unwrap();
    let _h0 = household(&c, 0).await;
    let _h1 = household(&c, 1).await;
    let dirs: Vec<_> = (0..3).map(|_| tempfile::tempdir().unwrap()).collect();
    let owner = open(&c, 0, dirs[0].path(), false).await;
    let friend = open(&c, 1, dirs[1].path(), false).await;
    let viewer = open(&c, 2, dirs[2].path(), false).await;

    let id = owner.create("Lisbon", "Ana's laptop").await.unwrap();
    let first = item(c.node(0), 1).await;
    let second = item(c.node(0), 2).await;
    owner
        .add_items(&id, vec![first.clone(), second.clone()])
        .await
        .unwrap();

    // A contributor from another household joins with the invitation text.
    let text = owner.invite(&id, MemberRole::Contributor, 7).await.unwrap();
    assert!(decode_invite(&text).is_ok());
    let joined = friend.join(&text, "Ben's phone").await.unwrap();
    assert_eq!(joined, id);
    friend.sync_now(&id).await.unwrap();
    let seen = friend.album(&id).unwrap().items(Page::all()).await.unwrap();
    assert_eq!(seen.len(), 2);

    // Previews and thumbnails come over, from a device that is not in the owner's device list.
    assert_eq!(friend.fetch_all(&id).await.unwrap(), 4);
    assert!(friend.has(&first.preview.hash).await.unwrap());
    assert!(
        !friend.has(&first.original.hash).await.unwrap(),
        "originals stay where they are"
    );
    friend.fetch_now(&id, &first.original.hash).await.unwrap();
    assert!(friend.has(&first.original.hash).await.unwrap());

    // The gate: a device that is neither in the library nor in the album gets nothing, and a
    // used invitation is refused.
    let secret = blob(c.node(0), 200, 3_000).await;
    assert!(
        c.node(2)
            .fetch_blob(c.node(0).id(), &secret.hash, Protection::Library)
            .await
            .is_err(),
        "a stranger cannot fetch blobs from an armed household"
    );
    let err = viewer.join(&text, "late").await.unwrap_err();
    assert!(err.to_string().contains("already been used"), "{err}");

    // The contributor adds a photo; the owner gets it.
    let third = item(c.node(1), 3).await;
    friend.add_items(&id, vec![third.clone()]).await.unwrap();
    owner.sync_now(&id).await.unwrap();
    assert_eq!(
        owner
            .album(&id)
            .unwrap()
            .items(Page::all())
            .await
            .unwrap()
            .len(),
        3
    );
    owner.fetch_all(&id).await.unwrap();
    assert!(owner.has(&third.thumb.hash).await.unwrap());

    // Names are shared with the members.
    let names: Vec<String> = owner
        .members(&id)
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.member.display_name)
        .collect();
    assert!(names.contains(&"Ana's laptop".to_string()), "{names:?}");
    assert!(names.contains(&"Ben's phone".to_string()), "{names:?}");

    // A viewer can look but not add (a read-only capability: nothing to write with).
    let vtext = owner.invite(&id, MemberRole::Viewer, 7).await.unwrap();
    viewer.join(&vtext, "TV").await.unwrap();
    viewer.sync_now(&id).await.unwrap();
    assert_eq!(
        viewer
            .album(&id)
            .unwrap()
            .items(Page::all())
            .await
            .unwrap()
            .len(),
        3
    );
    let err = viewer
        .add_items(&id, vec![item(c.node(2), 9).await])
        .await
        .unwrap_err();
    assert!(err.to_string().contains("only look"), "{err}");
    // Only owners invite.
    assert!(friend.invite(&id, MemberRole::Viewer, 1).await.is_err());

    // Everything reopens after a restart.
    drop(owner);
    let again = open(&c, 0, dirs[0].path(), false).await;
    let list = again.list().await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!((list[0].name.as_str(), list[0].item_count), ("Lisbon", 3));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invitations_expire_and_garbage_is_rejected_clearly() {
    let c = Cluster::new(2).await.unwrap();
    let dirs: Vec<_> = (0..2).map(|_| tempfile::tempdir().unwrap()).collect();
    let owner = open(&c, 0, dirs[0].path(), false).await;
    let other = open(&c, 1, dirs[1].path(), false).await;
    let id = owner.create("Old", "me").await.unwrap();
    let text = owner.invite(&id, MemberRole::Contributor, 7).await.unwrap();
    let mut t = decode_invite(&text).unwrap();
    t.expires = Expiry::At(Timestamp(1));
    let err = other.join(&encode_invite(&t), "x").await.unwrap_err();
    assert!(err.to_string().contains("expired"), "{err}");
    assert!(other
        .join("hello", "x")
        .await
        .unwrap_err()
        .to_string()
        .contains("not an album invitation"));
    assert!(other.join(&text[..text.len() - 6], "x").await.is_err());
    // Joining twice is refused.
    other.join(&text, "x").await.unwrap();
    assert!(other
        .join(&text, "x")
        .await
        .unwrap_err()
        .to_string()
        .contains("already"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_background_loop_keeps_members_up_to_date_and_keep_originals_fetches_them() {
    let c = Cluster::new(2).await.unwrap();
    let dirs: Vec<_> = (0..2).map(|_| tempfile::tempdir().unwrap()).collect();
    let owner = open(&c, 0, dirs[0].path(), true).await;
    let friend = open(&c, 1, dirs[1].path(), true).await;
    let id = owner.create("Live", "me").await.unwrap();
    let text = owner.invite(&id, MemberRole::Contributor, 7).await.unwrap();
    friend.join(&text, "you").await.unwrap();

    let it = item(c.node(0), 7).await;
    owner.add_items(&id, vec![it.clone()]).await.unwrap();
    let f = friend.clone();
    let hash: Blake3Hash = it.thumb.hash;
    wait_until(Duration::from_secs(40), || {
        let f = f.clone();
        async move { f.has(&hash).await.unwrap() }
    })
    .await
    .expect("thumbnail arrives on its own");

    friend.set_keep_originals(&id, true).unwrap();
    let orig = it.original.hash;
    wait_until(Duration::from_secs(40), || {
        let f = f.clone();
        async move { f.has(&orig).await.unwrap() }
    })
    .await
    .expect("original arrives once the album is kept in full");
}
