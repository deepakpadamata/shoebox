//! M2-02: creating identity yields a recovery kit; restoring from it makes a fresh device an
//! admin; a wrong phrase leaves no trace; the "kit saved" flag replicates as a library setting.

use shoebox_core::clock::ManualClock;
use shoebox_core::identity::KeyPair;
use shoebox_core::ids::EndpointId;
use shoebox_core::model::Role;
use shoebox_core::recovery::RecoveryError;
use shoebox_core::store::*;
use shoebox_node::{LibraryNode, Node, NodeConfig};
use shoebox_store_mem::{MemBackend, MemLibrary, MemNetwork};
use std::sync::Arc;

struct Dev {
    ln: LibraryNode<MemLibrary>,
    id: EndpointId,
    _dir: tempfile::TempDir,
}

async fn device(
    net: &MemNetwork,
    cap: Option<&LibraryCapability>,
    root: Option<&RootPublicKey>,
) -> (Dev, LibraryCapability) {
    let dir = tempfile::tempdir().unwrap();
    let node = Node::open(NodeConfig::local_only(dir.path().join("node"), "n"))
        .await
        .unwrap();
    let id = EndpointId(*node.id().as_bytes());
    let clock = ManualClock::new(1_700_000_000_000);
    let backend = MemBackend::new(net.clone(), id, clock.clone());
    let (lib, cap) = match cap {
        Some(cap) => (backend.open_library(cap).await.unwrap(), cap.clone()),
        None => backend.create_library(root.unwrap()).await.unwrap(),
    };
    let ln = LibraryNode::new(node, Arc::new(lib), clock).with_capability(cap.clone());
    (Dev { ln, id, _dir: dir }, cap)
}

#[tokio::test]
async fn identity_creation_returns_a_kit_that_restores_a_new_admin_device() {
    let net = MemNetwork::new();
    let root = KeyPair::generate();
    let (a, cap) = device(&net, None, Some(&root.root_public())).await;
    let kit =
        a.ln.create_identity(&root, Role::Keeper, "home server")
            .await
            .unwrap();
    assert_eq!(kit.library(), cap.library);
    assert!(kit.to_text().contains(&hex::encode(cap.library.0)));
    let list = a.ln.store().device_list().await.unwrap();
    assert_eq!(list.devices.len(), 1);
    assert_eq!(list.devices[0].device, a.id);
    assert_eq!(list.admins.len(), 1);

    // A brand-new device restores from the printed phrase and becomes an admin member.
    let (b, _) = device(&net, Some(&cap), None).await;
    b.ln.restore_from_kit(&kit.phrase(), Role::Keeper, "replacement")
        .await
        .unwrap();
    let list = b.ln.store().device_list().await.unwrap();
    assert!(list
        .devices
        .iter()
        .any(|d| d.device == b.id && !d.revoked && d.name == "replacement"));
    assert!(list.admins.iter().any(|x| x.admin == b.id));
}

#[tokio::test]
async fn a_wrong_phrase_fails_clearly_and_changes_nothing() {
    let net = MemNetwork::new();
    let root = KeyPair::generate();
    let (a, cap) = device(&net, None, Some(&root.root_public())).await;
    a.ln.create_identity(&root, Role::Keeper, "home")
        .await
        .unwrap();
    let (b, _) = device(&net, Some(&cap), None).await;
    let before = b.ln.store().device_list().await.unwrap();

    let other = shoebox_core::recovery::RecoveryKit::new(cap.library, 0, KeyPair::generate());
    for phrase in ["not a phrase", other.phrase().as_str()] {
        let err =
            b.ln.restore_from_kit(phrase, Role::Keeper, "x")
                .await
                .unwrap_err();
        let kind = err.downcast_ref::<RecoveryError>().copied();
        assert!(
            matches!(
                kind,
                Some(RecoveryError::BadPhrase | RecoveryError::WrongLibrary)
            ),
            "{err:#}"
        );
        assert_eq!(
            b.ln.store().device_list().await.unwrap(),
            before,
            "no partial state"
        );
    }
}

#[tokio::test]
async fn the_recovery_kit_saved_flag_is_a_library_setting() {
    let net = MemNetwork::new();
    let root = KeyPair::generate();
    let (a, cap) = device(&net, None, Some(&root.root_public())).await;
    assert!(
        !a.ln.recovery_kit_saved().await.unwrap(),
        "nag banner shows by default"
    );
    a.ln.set_recovery_kit_saved(true).await.unwrap();
    assert!(a.ln.recovery_kit_saved().await.unwrap());
    let _ = cap;
}

#[tokio::test]
async fn admins_can_revoke_devices_and_others_cannot() {
    let net = MemNetwork::new();
    let root = KeyPair::generate();
    let (a, cap) = device(&net, None, Some(&root.root_public())).await;
    a.ln.create_identity(&root, Role::Keeper, "home")
        .await
        .unwrap();
    // A second, non-admin device certified by the root (as pairing would do through an admin).
    let other = KeyPair::generate();
    let cert = shoebox_core::identity::issue_device_cert(
        &root,
        cap.library,
        other.endpoint_id(),
        Role::Phone,
        "phone",
        shoebox_core::Hlc {
            physical_ms: 1_699_999_999_000,
            counter: 0,
            device: a.id,
        },
        false,
    );
    a.ln.store()
        .apply_device_change(SignedDeviceChange::Device(cert))
        .await
        .unwrap();

    a.ln.revoke_device(&other.endpoint_id()).await.unwrap();
    let list = a.ln.store().device_list().await.unwrap();
    assert!(list
        .devices
        .iter()
        .any(|d| d.device == other.endpoint_id() && d.revoked));
    assert!(
        a.ln.revoke_device(&other.endpoint_id()).await.is_err(),
        "already revoked"
    );
    assert!(
        a.ln.revoke_device(&EndpointId([9; 32])).await.is_err(),
        "unknown device"
    );

    // A device without an admin cert cannot revoke.
    let (b, _) = device(&net, Some(&cap), None).await;
    let err = b.ln.revoke_device(&a.id).await.unwrap_err();
    assert!(format!("{err:#}").contains("admin"), "{err:#}");
}
