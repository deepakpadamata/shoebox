//! M2-03b: rotation notices are verifiable by anyone who knows the root key.

mod fixtures;

use fixtures::hlc;
use shoebox_core::identity::*;
use shoebox_core::model::Role;
use shoebox_core::store::{Capability, LibraryCapability, LibraryId};

const IDENTITY: LibraryId = LibraryId([7; 16]);
const NEW: LibraryId = LibraryId([8; 16]);

fn key(n: u8) -> KeyPair {
    KeyPair::from_seed([n; 32])
}

fn new_cap(root: &KeyPair) -> LibraryCapability {
    let mut backend = vec![2];
    backend.extend_from_slice(&IDENTITY.0);
    LibraryCapability {
        library: NEW,
        root: root.root_public(),
        mode: Capability::Write,
        backend,
    }
}

fn notice(root: &KeyPair, admin: &KeyPair, signer_is_admin: bool) -> RotationNotice {
    let admin_cert = issue_admin_cert(root, IDENTITY, admin.endpoint_id(), hlc(10, 0, 1), None);
    let signer = if signer_is_admin { admin } else { &key(9) };
    let revocation = issue_device_cert(
        signer,
        IDENTITY,
        key(3).endpoint_id(),
        Role::Phone,
        "lost",
        hlc(20, 0, 2),
        true,
    );
    issue_rotation_notice(
        admin,
        IDENTITY,
        IDENTITY,
        new_cap(root),
        revocation,
        admin_cert,
    )
}

#[test]
fn the_identity_id_survives_rotation() {
    let root = key(1);
    let cap = new_cap(&root);
    assert_eq!(cap.library, NEW);
    assert_eq!(cap.identity_id(), IDENTITY);
    let plain = LibraryCapability {
        library: NEW,
        root: root.root_public(),
        mode: Capability::Write,
        backend: vec![1],
    };
    assert_eq!(
        plain.identity_id(),
        NEW,
        "an unrotated library is its own identity"
    );
}

#[test]
fn a_valid_notice_verifies() {
    let (root, admin) = (key(1), key(2));
    assert_eq!(
        notice(&root, &admin, true).verify(&root.root_public(), IDENTITY),
        Ok(())
    );
}

#[test]
fn tampered_forged_and_misdirected_notices_fail() {
    let (root, admin) = (key(1), key(2));
    // The capability was swapped after signing.
    let mut n = notice(&root, &admin, true);
    n.new_cap.library = LibraryId([66; 16]);
    assert_eq!(
        n.verify(&root.root_public(), IDENTITY),
        Err(IdentityError::BadSignature)
    );
    // Signed by somebody who is not an admin.
    let mallory = key(9);
    let mut n = notice(&root, &admin, true);
    n.signature = mallory.sign(b"whatever");
    assert_eq!(
        n.verify(&root.root_public(), IDENTITY),
        Err(IdentityError::BadSignature)
    );
    // The revocation inside was not issued by an admin.
    let n = notice(&root, &admin, false);
    assert!(n.verify(&root.root_public(), IDENTITY).is_err());
    // Another library's root, or another identity.
    assert!(notice(&root, &admin, true)
        .verify(&key(5).root_public(), IDENTITY)
        .is_err());
    assert!(notice(&root, &admin, true)
        .verify(&root.root_public(), LibraryId([1; 16]))
        .is_err());
    // A notice that carries no revocation proves nothing.
    let mut n = notice(&root, &admin, true);
    n.revocation.revoked = false;
    assert!(n.verify(&root.root_public(), IDENTITY).is_err());
}

#[test]
fn a_root_handover_needs_both_roots_and_the_current_one() {
    use shoebox_core::store::{Capability, LibraryCapability, LibraryId};
    let identity = LibraryId([1; 16]);
    let (old_root, new_root, stranger) = (
        KeyPair::generate(),
        KeyPair::generate(),
        KeyPair::generate(),
    );
    let cap = |root: &KeyPair| LibraryCapability {
        library: LibraryId([2; 16]),
        root: root.root_public(),
        mode: Capability::Write,
        backend: [&[2u8][..], &identity.0[..]].concat(),
    };
    let h = issue_root_handover(
        &old_root,
        &new_root,
        identity,
        LibraryId([9; 16]),
        cap(&new_root),
    );
    assert_eq!(h.verify(&old_root.root_public(), identity), Ok(()));
    // Only from the root this device trusts now, and for this library.
    assert!(h.verify(&stranger.root_public(), identity).is_err());
    assert!(h
        .verify(&old_root.root_public(), LibraryId([7; 16]))
        .is_err());
    // Both signatures count: a handover to a key nobody holds, or a tampered one, fails.
    let mut no_new = h.clone();
    no_new.new_signature = stranger.sign(b"x");
    assert!(no_new.verify(&old_root.root_public(), identity).is_err());
    let mut moved = h.clone();
    moved.new_cap = cap(&stranger);
    assert!(moved.verify(&old_root.root_public(), identity).is_err());
    // Someone holding only a new key cannot hand over from the old one.
    let forged = issue_root_handover(
        &stranger,
        &new_root,
        identity,
        LibraryId([9; 16]),
        cap(&new_root),
    );
    assert!(forged.verify(&old_root.root_public(), identity).is_err());
}
