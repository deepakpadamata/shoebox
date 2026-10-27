//! M2-05: the keeper daemon — lock file, private control socket, pairing over the socket,
//! fetching and surviving a restart.

use keeper::control::Client;
use keeper::{Daemon, KeeperOptions};
use serde_json::Value;
use shoebox_core::clock::SystemClock;
use shoebox_core::identity::KeyPair;
use shoebox_core::model::Role;
use shoebox_core::store::StoreBackend;
use shoebox_node::{ImportOptions, LibraryNode, Node, NodeConfig};
use shoebox_store::{HybridBackend, Library};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

fn opts(dir: &Path) -> KeeperOptions {
    KeeperOptions::local_only(dir.join("keeper"), "pi")
}

async fn wait_for<F: FnMut() -> Fut, Fut: std::future::Future<Output = bool>>(
    secs: u64,
    mut f: F,
) -> bool {
    let end = std::time::Instant::now() + Duration::from_secs(secs);
    while std::time::Instant::now() < end {
        if f().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

/// A phone-like admin device with a library holding `files`.
async fn inviter(dir: &Path, files: &[&str]) -> (Node, LibraryNode<Library>) {
    let node = Node::open(NodeConfig::local_only(dir.join("inviter"), "laptop"))
        .await
        .unwrap();
    let backend = HybridBackend::for_node(node.clone(), SystemClock);
    let root = KeyPair::generate();
    let (lib, cap) = backend.create_library(&root.root_public()).await.unwrap();
    let ln = LibraryNode::new(node.clone(), Arc::new(lib), SystemClock).with_capability(cap);
    ln.create_identity(&root, Role::Companion, "laptop")
        .await
        .unwrap();
    let samples: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples");
    let paths: Vec<PathBuf> = files.iter().map(|f| samples.join(f)).collect();
    if !paths.is_empty() {
        let report = ln
            .import_paths(&paths, ImportOptions::default())
            .await
            .unwrap();
        assert_eq!(report.imported.len(), paths.len(), "{report:?}");
    }
    (node, ln)
}

fn accept() -> shoebox_node::DecideFn {
    Arc::new(|_| Box::pin(async { true }))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_second_daemon_on_the_same_storage_refuses_to_start() {
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(opts(dir.path())).await.unwrap();
    let err = Daemon::start(opts(dir.path()))
        .await
        .err()
        .expect("second start fails");
    assert!(format!("{err:#}").contains("already running"), "{err:#}");
    d.shutdown().await.unwrap();
    // After a clean shutdown the storage can be used again.
    Daemon::start(opts(dir.path()))
        .await
        .unwrap()
        .shutdown()
        .await
        .unwrap();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_control_socket_is_private_and_reports_an_unpaired_keeper() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let d = Daemon::start(opts(dir.path())).await.unwrap();
    let mode = std::fs::metadata(d.control_path())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    let key_mode = std::fs::metadata(dir.path().join("keeper/keys/endpoint.key"))
        .map(|m| m.permissions().mode() & 0o777);
    if let Ok(m) = key_mode {
        assert_eq!(m, 0o600, "endpoint secret file");
    }
    let mut c = Client::connect(d.control_path()).await.unwrap();
    let status = c.status().await.unwrap();
    assert_eq!(status["paired"], Value::Bool(false));
    assert_eq!(status["name"], "pi");
    let err = c
        .call_raw(serde_json::json!({"cmd":"devices"}))
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("not paired"), "{err:#}");
    let err = c
        .call_raw(serde_json::json!({"cmd":"nonsense"}))
        .await
        .unwrap_err();
    assert!(
        format!("{err:#}").to_lowercase().contains("unknown"),
        "{err:#}"
    );
    d.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pairing_over_the_socket_joins_the_library_and_fetches_originals() {
    let dir = tempfile::tempdir().unwrap();
    let (_inode, ln) = inviter(dir.path(), &["photo-gps.jpg", "IMG_0001.jpg"]).await;
    let ticket = ln
        .start_pairing(Role::Keeper, Duration::from_secs(300), accept())
        .await
        .unwrap();

    let d = Daemon::start(opts(dir.path())).await.unwrap();
    let mut c = Client::connect(d.control_path()).await.unwrap();
    let mut shown = None;
    c.pair(&ticket.to_string(), "pi", |code| {
        shown = Some(code.sas.clone());
        true
    })
    .await
    .unwrap();
    assert_eq!(shown.unwrap().len(), 6);

    assert!(
        wait_for(30, || async {
            let s = Client::connect(d.control_path())
                .await
                .unwrap()
                .status()
                .await
                .unwrap();
            s["paired"] == true
                && s["library"]["photos"] == 2
                && s["library"]["held_originals"] == 2
        })
        .await,
        "the keeper syncs the library and fetches both originals"
    );
    let devices = Client::connect(d.control_path())
        .await
        .unwrap()
        .call_raw(serde_json::json!({"cmd":"devices"}))
        .await
        .unwrap();
    let names: Vec<&str> = devices["devices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"laptop") && names.contains(&"pi"),
        "{names:?}"
    );
    d.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restarted_keeper_stays_paired_and_finishes_fetching() {
    let dir = tempfile::tempdir().unwrap();
    let (_inode, ln) = inviter(
        dir.path(),
        &["photo-gps.jpg", "IMG_0001.jpg", "screenshot.png"],
    )
    .await;
    let ticket = ln
        .start_pairing(Role::Keeper, Duration::from_secs(300), accept())
        .await
        .unwrap();

    let d = Daemon::start(opts(dir.path())).await.unwrap();
    Client::connect(d.control_path())
        .await
        .unwrap()
        .pair(&ticket.to_string(), "pi", |_| true)
        .await
        .unwrap();
    // Stop right away, most likely before every original arrived.
    d.shutdown().await.unwrap();

    let d = Daemon::start(opts(dir.path())).await.unwrap();
    assert!(
        wait_for(40, || async {
            let s = Client::connect(d.control_path())
                .await
                .unwrap()
                .status()
                .await
                .unwrap();
            s["paired"] == true
                && s["library"]["photos"] == 3
                && s["library"]["held_originals"] == 3
        })
        .await,
        "after the restart the keeper is still paired and completes its fetch"
    );
    d.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_keeper_follows_a_rotation_and_rejoins_the_new_library_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let (_inode, ln) = inviter(dir.path(), &["photo-gps.jpg"]).await;
    let ticket = ln
        .start_pairing(Role::Keeper, Duration::from_secs(300), accept())
        .await
        .unwrap();
    let d = Daemon::start(opts(dir.path())).await.unwrap();
    Client::connect(d.control_path())
        .await
        .unwrap()
        .pair(&ticket.to_string(), "pi", |_| true)
        .await
        .unwrap();

    // A second device joins, then the admin revokes it and rotates the library.
    let phone = Node::open(NodeConfig::local_only(dir.path().join("phone"), "phone"))
        .await
        .unwrap();
    let t = ln
        .start_pairing(Role::Companion, Duration::from_secs(300), accept())
        .await
        .unwrap();
    phone.pair_join(&t, "phone", accept()).await.unwrap();
    let phone_id = shoebox_core::ids::EndpointId(*phone.id().as_bytes());
    let before = ln.library_id();
    ln.revoke_and_rotate(&phone_id).await.unwrap();
    assert_ne!(ln.library_id(), before);

    let want = hex::encode(ln.library_id().0);
    assert!(
        wait_for(30, || async {
            let s = Client::connect(d.control_path())
                .await
                .unwrap()
                .status()
                .await
                .unwrap();
            s["library"]["id"] == want.as_str() && s["library"]["photos"] == 1
        })
        .await,
        "the keeper follows the rotation and syncs the new epoch"
    );
    d.shutdown().await.unwrap();
    phone.shutdown().await.ok();

    // After a restart it rejoins the *new* library, not the old one.
    let d = Daemon::start(opts(dir.path())).await.unwrap();
    assert!(
        wait_for(30, || async {
            let s = Client::connect(d.control_path())
                .await
                .unwrap()
                .status()
                .await
                .unwrap();
            s["library"]["id"] == want.as_str()
                && s["library"]["photos"] == 1
                && s["membership"] == "member"
        })
        .await,
        "a restarted keeper is on the new epoch"
    );
    d.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unpaired_keeper_recovers_with_the_phrase_through_a_surviving_device() {
    let dir = tempfile::tempdir().unwrap();
    let node = Node::open(NodeConfig::local_only(
        dir.path().join("survivor"),
        "laptop",
    ))
    .await
    .unwrap();
    let backend = HybridBackend::for_node(node.clone(), SystemClock);
    let root = KeyPair::generate();
    let (lib, cap) = backend.create_library(&root.root_public()).await.unwrap();
    let ln = LibraryNode::new(node.clone(), Arc::new(lib), SystemClock).with_capability(cap);
    let kit = ln
        .create_identity(&root, Role::Companion, "laptop")
        .await
        .unwrap();
    let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples");
    ln.import_paths(&[samples.join("photo-gps.jpg")], ImportOptions::default())
        .await
        .unwrap();

    let d = Daemon::start(opts(dir.path())).await.unwrap();
    let mut c = Client::connect(d.control_path()).await.unwrap();
    let link = ln.peer_link().to_string();
    let err = c
        .call_raw(
            serde_json::json!({"cmd":"recover","from":link,"phrase":"wrong words","name":"pi"}),
        )
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("24-word"), "{err:#}");
    let done = c
        .call_raw(
            serde_json::json!({"cmd":"recover","from":link,"phrase":kit.phrase(),"name":"pi"}),
        )
        .await
        .unwrap();
    assert_eq!(done["library"].as_str().unwrap().len(), 32);
    assert!(
        wait_for(30, || async {
            let s = Client::connect(d.control_path())
                .await
                .unwrap()
                .status()
                .await
                .unwrap();
            s["paired"] == true
                && s["library"]["photos"] == 1
                && s["library"]["held_originals"] == 1
        })
        .await,
        "the recovered keeper syncs the library and fetches the original"
    );
    // It can print its own link for the next recovery.
    let a = Client::connect(d.control_path())
        .await
        .unwrap()
        .call_raw(serde_json::json!({"cmd":"address"}))
        .await
        .unwrap();
    assert!(a["link"].as_str().unwrap().starts_with("shoebox-peer1"));
    d.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_keeper_joins_an_album_and_keeps_every_original_when_told_to() {
    use shoebox_core::model::SharedItem;
    let dir = tempfile::tempdir().unwrap();
    let (inode, ln) = inviter(dir.path(), &["photo-gps.jpg", "IMG_0001.jpg"]).await;
    let ticket = ln
        .start_pairing(Role::Keeper, Duration::from_secs(300), accept())
        .await
        .unwrap();
    let d = Daemon::start(opts(dir.path())).await.unwrap();
    Client::connect(d.control_path())
        .await
        .unwrap()
        .pair(&ticket.to_string(), "pi", |_| true)
        .await
        .unwrap();

    // The laptop shares one photo in an album and invites the keeper.
    let albums =
        shoebox_store::SharedAlbums::open(inode.clone(), &dir.path().join("laptop-albums"), true)
            .await
            .unwrap();
    std::fs::create_dir_all(dir.path().join("laptop-albums")).ok();
    let id = albums.create("Trip", "laptop").await.unwrap();
    // A photo that is not in the keeper's library (so the library does not bring it along).
    let mk = |seed: u8, len: usize| {
        let node = inode.clone();
        async move {
            let hash = node
                .blobs()
                .add_bytes(vec![seed; len], shoebox_node::Protection::Library)
                .await
                .unwrap();
            shoebox_core::model::BlobRef {
                hash,
                size: len as u64,
                mime: "image/jpeg".into(),
            }
        }
    };
    let original = mk(1, 20_000).await;
    let item = SharedItem {
        id: original.hash,
        original,
        shared_hash: None,
        preview: mk(2, 900).await,
        thumb: mk(3, 100).await,
        added_by: ln.device(),
        added_at: shoebox_core::Timestamp(1),
        taken_at: shoebox_core::Timestamp(0),
    };
    albums.add_items(&id, vec![item.clone()]).await.unwrap();
    let invite = albums
        .invite(&id, shoebox_core::model::MemberRole::Contributor, 7)
        .await
        .unwrap();

    let mut c = Client::connect(d.control_path()).await.unwrap();
    let joined = c
        .call_raw(serde_json::json!({"cmd":"album","action":"join","text":invite}))
        .await
        .unwrap();
    assert_eq!(joined["name"], "Trip");
    let list = c
        .call_raw(serde_json::json!({"cmd":"album","action":"list"}))
        .await
        .unwrap();
    assert_eq!(list["albums"][0]["keep_originals"], false);
    let short = &list["albums"][0]["id"].as_str().unwrap()[..6].to_string();
    // Previews arrive on their own; the original only once the keeper is told to keep it all.
    let preview = item.preview.hash;
    assert!(
        wait_for(30, || async {
            d.node().blobs().has(&preview).await.unwrap()
        })
        .await,
        "preview arrives"
    );
    assert!(
        !d.node().blobs().has(&item.original.hash).await.unwrap(),
        "originals are on demand until the keeper is told to keep them"
    );
    c.call_raw(serde_json::json!({"cmd":"album","action":"keep","id":short}))
        .await
        .unwrap();
    let original = item.original.hash;
    assert!(
        wait_for(30, || async {
            d.node().blobs().has(&original).await.unwrap()
        })
        .await,
        "the original arrives once the album is kept in full"
    );
    let err = c
        .call_raw(serde_json::json!({"cmd":"album","action":"keep","id":"zzzz"}))
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("no album"), "{err:#}");
    let status = c.status().await.unwrap();
    assert_eq!(status["shared_albums"][0]["keep_originals"], true);
    d.shutdown().await.unwrap();
}

/// R1-06: the keeper has no screen, so a security alert (here: the recovery phrase was replaced on
/// another device) waits in `status` until someone acknowledges it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn security_alerts_wait_in_status_until_acknowledged() {
    let dir = tempfile::tempdir().unwrap();
    let node = Node::open(NodeConfig::local_only(dir.path().join("inviter"), "laptop"))
        .await
        .unwrap();
    let backend = HybridBackend::for_node(node.clone(), SystemClock);
    let root = KeyPair::generate();
    let (lib, cap) = backend.create_library(&root.root_public()).await.unwrap();
    let ln = LibraryNode::new(node.clone(), Arc::new(lib), SystemClock).with_capability(cap);
    let kit = ln
        .create_identity(&root, Role::Companion, "laptop")
        .await
        .unwrap();
    ln.enforce_membership().await.unwrap();
    let ticket = ln
        .start_pairing(Role::Keeper, Duration::from_secs(300), accept())
        .await
        .unwrap();
    let d = Daemon::start(opts(dir.path())).await.unwrap();
    Client::connect(d.control_path())
        .await
        .unwrap()
        .pair(&ticket.to_string(), "pi", |_| true)
        .await
        .unwrap();
    let status = || async {
        Client::connect(d.control_path())
            .await
            .unwrap()
            .status()
            .await
            .unwrap()
    };
    assert!(
        wait_for(30, || async { status().await["paired"] == true }).await,
        "paired"
    );
    // Let it finish its first sync (it alerts from then on).
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(status().await["alerts"], serde_json::json!([]));

    ln.rotate_root(&kit.phrase()).await.unwrap();
    assert!(
        wait_for(30, || async {
            status().await["alerts"]
                .as_array()
                .is_some_and(|a| a.iter().any(|x| x["kind"] == "phrase_replaced"))
        })
        .await,
        "the keeper reports that the phrase was replaced"
    );
    let text = keeper::render::status(&status().await);
    assert!(text.contains("SECURITY ALERTS"), "{text}");

    Client::connect(d.control_path())
        .await
        .unwrap()
        .call_raw(serde_json::json!({"cmd":"alerts_ack"}))
        .await
        .unwrap();
    assert_eq!(status().await["alerts"], serde_json::json!([]));
    d.shutdown().await.unwrap();
}

/// R1-07: the keeper reads the release feed and `status` mentions a newer version.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_mentions_a_newer_release() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let feed = format!(
        "http://{}/downloads/release.json",
        listener.local_addr().unwrap()
    );
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf).await;
                let body = r#"{"version":"99.1.0"}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = s.write_all(resp.as_bytes()).await;
            });
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let mut o = opts(dir.path());
    o.update_feed = feed;
    let d = Daemon::start(o).await.unwrap();
    assert!(
        wait_for(10, || async {
            let s = Client::connect(d.control_path())
                .await
                .unwrap()
                .status()
                .await
                .unwrap();
            s["update"]["version"] == "99.1.0"
        })
        .await,
        "status mentions 99.1.0"
    );
    let s = Client::connect(d.control_path())
        .await
        .unwrap()
        .status()
        .await
        .unwrap();
    assert!(keeper::render::status(&s).contains("A newer version is available: 99.1.0"));
    d.shutdown().await.unwrap();
}

/// The keeper's command line can do what an admin can: change roles, make an admin, remove a
/// device and replace the recovery phrase, with or without the phrase as the rules say. Devices
/// are named by id, the start of an id or their name.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_keeper_manages_devices_from_its_command_line() {
    let dir = tempfile::tempdir().unwrap();
    let node = Node::open(NodeConfig::local_only(dir.path().join("laptop"), "laptop"))
        .await
        .unwrap();
    let backend = HybridBackend::for_node(node.clone(), SystemClock);
    let root = KeyPair::generate();
    let (lib, cap) = backend.create_library(&root.root_public()).await.unwrap();
    let ln = LibraryNode::new(node.clone(), Arc::new(lib), SystemClock).with_capability(cap);
    let kit = ln
        .create_identity(&root, Role::Companion, "laptop")
        .await
        .unwrap();
    let phrase = kit.phrase();

    let ticket = ln
        .start_pairing(Role::Keeper, Duration::from_secs(300), accept())
        .await
        .unwrap();
    let d = Daemon::start(opts(dir.path())).await.unwrap();
    Client::connect(d.control_path())
        .await
        .unwrap()
        .pair(&ticket.to_string(), "pi", |_| true)
        .await
        .unwrap();
    let phone = Node::open(NodeConfig::local_only(dir.path().join("phone"), "phone"))
        .await
        .unwrap();
    let t = ln
        .start_pairing(Role::Companion, Duration::from_secs(300), accept())
        .await
        .unwrap();
    phone.pair_join(&t, "phone", accept()).await.unwrap();

    let call = |req: Value| {
        let path = d.control_path().to_path_buf();
        async move { Client::connect(&path).await.unwrap().call_raw(req).await }
    };
    let row = |name: &'static str| {
        let path = d.control_path().to_path_buf();
        async move {
            let v = Client::connect(&path)
                .await
                .unwrap()
                .call_raw(serde_json::json!({"cmd":"devices"}))
                .await
                .unwrap();
            v["devices"]
                .as_array()
                .unwrap()
                .iter()
                .find(|d| d["name"] == name)
                .cloned()
        }
    };
    assert!(
        wait_for(30, || async { row("phone").await.is_some() }).await,
        "the keeper hears about the phone"
    );

    // A plain member cannot change roles; the wrong phrase does not help.
    let err = call(serde_json::json!({"cmd":"role","device":"phone","role":"viewer"}))
        .await
        .unwrap_err();
    assert!(
        format!("{err:#}").to_lowercase().contains("admin"),
        "{err:#}"
    );
    let err = call(
        serde_json::json!({"cmd":"role","device":"phone","role":"viewer","phrase":"wrong words"}),
    )
    .await
    .unwrap_err();
    assert!(
        format!("{err:#}").contains("24-word") || format!("{err:#}").contains("phrase"),
        "{err:#}"
    );
    // With the phrase it can, naming the device.
    call(serde_json::json!({"cmd":"role","device":"phone","role":"viewer","phrase":phrase}))
        .await
        .unwrap();
    assert!(
        wait_for(30, || async {
            row("phone").await.is_some_and(|r| r["role"] == "viewer")
        })
        .await,
        "the role change reaches the keeper's view"
    );
    let err =
        call(serde_json::json!({"cmd":"role","device":"nobody","role":"viewer","phrase":phrase}))
            .await
            .unwrap_err();
    assert!(format!("{err:#}").contains("no device matches"), "{err:#}");
    let err =
        call(serde_json::json!({"cmd":"role","device":"phone","role":"boss","phrase":phrase}))
            .await
            .unwrap_err();
    assert!(
        format!("{err:#}").contains("keeper, companion or viewer"),
        "{err:#}"
    );

    // Making the keeper an admin needs the phrase; after that it needs none.
    let err = call(serde_json::json!({"cmd":"make_admin","device":"pi"}))
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("phrase"), "{err:#}");
    let pi = row("pi").await.unwrap();
    let id = pi["id"].as_str().unwrap().to_string();
    call(serde_json::json!({"cmd":"make_admin","device":&id[..8],"phrase":phrase}))
        .await
        .unwrap();
    assert!(
        wait_for(30, || async {
            row("pi").await.is_some_and(|r| r["admin"] == true)
        })
        .await,
        "the keeper is an admin"
    );
    call(serde_json::json!({"cmd":"role","device":"phone","role":"companion"}))
        .await
        .unwrap();
    assert!(
        wait_for(30, || async {
            row("phone").await.is_some_and(|r| r["role"] == "companion")
        })
        .await,
        "an admin keeper changes a role without the phrase"
    );

    // Removing a device moves the library to a new epoch.
    let before = Client::connect(d.control_path())
        .await
        .unwrap()
        .status()
        .await
        .unwrap()["library"]["id"]
        .clone();
    call(serde_json::json!({"cmd":"revoke","device":"phone"}))
        .await
        .unwrap();
    assert!(
        wait_for(30, || async {
            row("phone").await.is_some_and(|r| r["revoked"] == true)
        })
        .await,
        "the phone is removed"
    );
    assert!(
        wait_for(30, || async {
            Client::connect(d.control_path())
                .await
                .unwrap()
                .status()
                .await
                .unwrap()["library"]["id"]
                != before
        })
        .await,
        "removing a device moves the library to a new epoch"
    );

    // Replacing the phrase needs the current one and retires it.
    let err = call(serde_json::json!({"cmd":"replace_phrase","phrase":"wrong words"}))
        .await
        .unwrap_err();
    assert!(!format!("{err:#}").is_empty());
    let new = call(serde_json::json!({"cmd":"replace_phrase","phrase":phrase}))
        .await
        .unwrap();
    let new_phrase = new["phrase"].as_str().unwrap().to_string();
    assert_eq!(new_phrase.split_whitespace().count(), 24);
    assert_ne!(new_phrase, phrase);
    call(serde_json::json!({"cmd":"replace_phrase","phrase":phrase}))
        .await
        .unwrap_err();
    d.shutdown().await.unwrap();
    phone.shutdown().await.ok();
}
