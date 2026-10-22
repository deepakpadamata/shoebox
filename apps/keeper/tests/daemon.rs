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
    ln.create_identity(&root, Role::Phone, "laptop")
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
