//! M3-02: the endpoint secret lives in a keystore, not under the app's data directory.

use desktop_mobile::keystore::{migrate_file_key, FileStore, SecretStore};
use desktop_mobile::{AppCore, AppOptions};
use std::path::Path;
use std::sync::Arc;

fn files_named_key(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(files_named_key(&p));
        } else if p.extension().is_some_and(|x| x == "key") {
            out.push(p);
        }
    }
    out
}

#[test]
fn the_file_store_creates_once_and_keeps_the_secret_private() {
    let dir = tempfile::tempdir().unwrap();
    let store = FileStore::new(dir.path());
    assert_eq!(store.load("k").unwrap(), None);
    let a = store.load_or_create("k").unwrap();
    assert_eq!(store.load_or_create("k").unwrap(), a, "stable across calls");
    assert_ne!(store.load_or_create("other").unwrap(), a);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join("k.key"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
    std::fs::write(dir.path().join("broken.key"), b"short").unwrap();
    assert!(
        store.load("broken").is_err(),
        "a damaged secret is an error, not a new key"
    );
}

#[test]
fn migration_moves_the_dev_key_into_the_store_and_wipes_the_file() {
    let node_dir = tempfile::tempdir().unwrap();
    let keys = node_dir.path().join("keys");
    std::fs::create_dir_all(&keys).unwrap();
    std::fs::write(keys.join("endpoint.key"), [7u8; 32]).unwrap();
    let store_dir = tempfile::tempdir().unwrap();
    let store = FileStore::new(store_dir.path());

    assert!(migrate_file_key(node_dir.path(), &store, "endpoint-key").unwrap());
    assert_eq!(store.load("endpoint-key").unwrap(), Some([7u8; 32]));
    assert!(
        files_named_key(node_dir.path()).is_empty(),
        "no secret left in the data dir"
    );
    assert!(
        !migrate_file_key(node_dir.path(), &store, "endpoint-key").unwrap(),
        "nothing left to migrate"
    );

    // A different key already in the store is never overwritten (that would change the identity).
    std::fs::write(keys.join("endpoint.key"), [9u8; 32]).unwrap();
    assert!(migrate_file_key(node_dir.path(), &store, "endpoint-key").is_err());
    assert_eq!(store.load("endpoint-key").unwrap(), Some([7u8; 32]));
    std::fs::write(keys.join("endpoint.key"), b"junk").unwrap();
    assert!(migrate_file_key(node_dir.path(), &store, "x").is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_app_with_a_keystore_has_a_stable_identity_and_no_key_file_in_its_data_dir() {
    let data = tempfile::tempdir().unwrap();
    let secrets = tempfile::tempdir().unwrap();
    let store: Arc<dyn SecretStore> = Arc::new(FileStore::new(secrets.path()));
    let opts = || AppOptions {
        secrets: Some(store.clone()),
        ..AppOptions::local_only()
    };

    let app = AppCore::open(data.path(), opts()).await.unwrap();
    let id = app.app_state().await.unwrap().device_id;
    app.shutdown().await.unwrap();
    assert!(
        files_named_key(data.path()).is_empty(),
        "the data dir holds no secret: {:?}",
        files_named_key(data.path())
    );

    let again = AppCore::open(data.path(), opts()).await.unwrap();
    assert_eq!(
        again.app_state().await.unwrap().device_id,
        id,
        "same keystore, same device"
    );
    again.shutdown().await.unwrap();
    assert_eq!(
        store.load("endpoint-key").unwrap().map(|k| k.len()),
        Some(32)
    );
}

#[test]
fn migrating_from_the_file_store_keeps_the_identity_and_wipes_the_file() {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (files, keystore) = (FileStore::new(a.path()), FileStore::new(b.path()));
    let secret = files.load_or_create("endpoint-key").unwrap();
    assert!(
        desktop_mobile::keystore::migrate_from_files(&files, &keystore, "endpoint-key").unwrap()
    );
    assert_eq!(
        keystore.load("endpoint-key").unwrap(),
        Some(secret),
        "same identity"
    );
    assert!(
        files_named_key(a.path()).is_empty(),
        "no secret left in the old place"
    );
    assert!(
        !desktop_mobile::keystore::migrate_from_files(&files, &keystore, "endpoint-key").unwrap()
    );
    // A different key already in the keystore is never overwritten.
    files.store("endpoint-key", &[1u8; 32]).unwrap();
    assert!(
        desktop_mobile::keystore::migrate_from_files(&files, &keystore, "endpoint-key").is_err()
    );
    assert_eq!(keystore.load("endpoint-key").unwrap(), Some(secret));
}
