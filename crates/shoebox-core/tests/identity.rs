//! M2-01: the trust chain root → admin → device, revocation, expiry and forgery.

mod fixtures;

use fixtures::hlc;
use shoebox_core::identity::*;
use shoebox_core::model::Role;
use shoebox_core::store::{LibraryId, RootPublicKey};
use shoebox_core::{EndpointId, Timestamp};

const LIB: LibraryId = LibraryId([7; 16]);

fn key(n: u8) -> KeyPair {
    KeyPair::from_seed([n; 32])
}

fn chain(root: &KeyPair) -> TrustChain {
    TrustChain::new(root.public(), LIB)
}

#[test]
fn keys_are_deterministic_and_a_device_id_is_its_public_key() {
    assert_eq!(key(1).public_bytes(), key(1).public_bytes());
    assert_ne!(key(1).public_bytes(), key(2).public_bytes());
    assert_eq!(key(1).endpoint_id(), EndpointId(key(1).public_bytes()));
    assert_eq!(key(9).root_public(), RootPublicKey(key(9).public_bytes()));
    let sig = key(1).sign(b"hello");
    assert!(verify_signature(&key(1).public_bytes(), b"hello", &sig));
    assert!(!verify_signature(&key(1).public_bytes(), b"hellO", &sig));
    assert!(!verify_signature(&key(2).public_bytes(), b"hello", &sig));
    assert!(
        !verify_signature(&key(1).public_bytes(), b"hello", &sig[..10]),
        "short signature"
    );
    assert!(
        !verify_signature(&[0xFF; 32], b"hello", &sig),
        "not a valid public key"
    );
}

#[test]
fn generated_keys_differ() {
    assert_ne!(
        KeyPair::generate().public_bytes(),
        KeyPair::generate().public_bytes()
    );
}

#[test]
fn a_valid_chain_is_accepted() {
    let (root, admin, phone) = (key(1), key(2), key(3));
    let mut c = chain(&root);
    let admin_cert = issue_admin_cert(&root, LIB, admin.endpoint_id(), hlc(10, 0, 1), None);
    assert_eq!(c.check_admin(&admin_cert), Ok(()));
    c.add_admin(admin_cert);
    let cert = issue_device_cert(
        &admin,
        LIB,
        phone.endpoint_id(),
        Role::Companion,
        "Asha's phone",
        hlc(20, 0, 2),
        false,
    );
    assert_eq!(c.check_device_cert(&cert), Ok(()));
    c.add_device(cert.clone());
    let valid = c.validate();
    assert_eq!(valid.admins.len(), 1);
    assert_eq!(valid.devices, vec![cert]);
    assert!(valid.is_member(&phone.endpoint_id()));
    assert!(!valid.is_member(&key(9).endpoint_id()));
}

#[test]
fn the_root_can_certify_a_device_directly() {
    let (root, first) = (key(1), key(2));
    let c = chain(&root);
    let cert = issue_device_cert(
        &root,
        LIB,
        first.endpoint_id(),
        Role::Keeper,
        "first",
        hlc(5, 0, 1),
        false,
    );
    assert_eq!(c.check_device_cert(&cert), Ok(()));
}

#[test]
fn forged_and_tampered_certs_are_rejected() {
    let (root, admin, phone, mallory) = (key(1), key(2), key(3), key(4));
    let mut c = chain(&root);
    c.add_admin(issue_admin_cert(
        &root,
        LIB,
        admin.endpoint_id(),
        hlc(10, 0, 1),
        None,
    ));

    // Admin cert signed by someone other than the root.
    let forged_admin = issue_admin_cert(&mallory, LIB, mallory.endpoint_id(), hlc(11, 0, 4), None);
    assert_eq!(
        c.check_admin(&forged_admin),
        Err(IdentityError::BadSignature)
    );

    // A device cert whose fields were changed after signing.
    let mut cert = issue_device_cert(
        &admin,
        LIB,
        phone.endpoint_id(),
        Role::Companion,
        "phone",
        hlc(20, 0, 2),
        false,
    );
    cert.role = Role::Keeper;
    assert_eq!(c.check_device_cert(&cert), Err(IdentityError::BadSignature));
    let mut cert = issue_device_cert(
        &admin,
        LIB,
        phone.endpoint_id(),
        Role::Companion,
        "phone",
        hlc(20, 0, 2),
        false,
    );
    cert.name = "something else".into();
    assert_eq!(c.check_device_cert(&cert), Err(IdentityError::BadSignature));

    // Claiming to be signed by the admin while signed by mallory.
    let mut cert = issue_device_cert(
        &mallory,
        LIB,
        phone.endpoint_id(),
        Role::Companion,
        "phone",
        hlc(20, 0, 2),
        false,
    );
    cert.added_by = admin.endpoint_id();
    assert_eq!(c.check_device_cert(&cert), Err(IdentityError::BadSignature));

    // Empty or garbage signatures.
    let mut cert = issue_device_cert(
        &admin,
        LIB,
        phone.endpoint_id(),
        Role::Companion,
        "phone",
        hlc(20, 0, 2),
        false,
    );
    cert.signature.clear();
    assert_eq!(c.check_device_cert(&cert), Err(IdentityError::BadSignature));

    // A cert minted for another library does not verify here.
    let other = issue_device_cert(
        &admin,
        LibraryId([8; 16]),
        phone.endpoint_id(),
        Role::Companion,
        "phone",
        hlc(20, 0, 2),
        false,
    );
    assert_eq!(
        c.check_device_cert(&other),
        Err(IdentityError::BadSignature)
    );

    // Admin certs from a different root are not accepted either.
    let other_root = key(5);
    let c2 = chain(&other_root);
    let cert = issue_admin_cert(&root, LIB, admin.endpoint_id(), hlc(10, 0, 1), None);
    assert_eq!(c2.check_admin(&cert), Err(IdentityError::BadSignature));
}

#[test]
fn only_admins_can_certify_devices() {
    let (root, admin, regular, newcomer) = (key(1), key(2), key(3), key(4));
    let mut c = chain(&root);
    c.add_admin(issue_admin_cert(
        &root,
        LIB,
        admin.endpoint_id(),
        hlc(10, 0, 1),
        None,
    ));
    // `regular` is a device but holds no admin cert.
    let cert = issue_device_cert(
        &regular,
        LIB,
        newcomer.endpoint_id(),
        Role::Companion,
        "n",
        hlc(20, 0, 3),
        false,
    );
    assert_eq!(
        c.check_device_cert(&cert),
        Err(IdentityError::SignerNotAdmin)
    );
    // The same device with a delegation can.
    c.add_admin(issue_admin_cert(
        &root,
        LIB,
        regular.endpoint_id(),
        hlc(15, 0, 1),
        None,
    ));
    assert_eq!(c.check_device_cert(&cert), Ok(()));
}

#[test]
fn an_admin_cert_that_expired_before_signing_does_not_authorise_it() {
    let (root, admin, phone) = (key(1), key(2), key(3));
    let mut c = chain(&root);
    c.add_admin(issue_admin_cert(
        &root,
        LIB,
        admin.endpoint_id(),
        hlc(10, 0, 1),
        Some(Timestamp(100)),
    ));
    let before = issue_device_cert(
        &admin,
        LIB,
        phone.endpoint_id(),
        Role::Companion,
        "p",
        hlc(99, 0, 2),
        false,
    );
    let after = issue_device_cert(
        &admin,
        LIB,
        phone.endpoint_id(),
        Role::Companion,
        "p",
        hlc(100, 0, 2),
        false,
    );
    assert_eq!(
        c.check_device_cert(&before),
        Ok(()),
        "valid while the delegation was valid"
    );
    assert_eq!(c.check_device_cert(&after), Err(IdentityError::Expired));
    // An admin cert cannot be used before it was issued either.
    let early = issue_device_cert(
        &admin,
        LIB,
        phone.endpoint_id(),
        Role::Companion,
        "p",
        hlc(5, 0, 2),
        false,
    );
    assert_eq!(
        c.check_device_cert(&early),
        Err(IdentityError::SignerNotAdmin)
    );
}

#[test]
fn revocation_needs_an_admin_or_the_root() {
    let (root, admin, phone, mallory) = (key(1), key(2), key(3), key(4));
    let mut c = chain(&root);
    c.add_admin(issue_admin_cert(
        &root,
        LIB,
        admin.endpoint_id(),
        hlc(10, 0, 1),
        None,
    ));
    c.add_device(issue_device_cert(
        &admin,
        LIB,
        phone.endpoint_id(),
        Role::Companion,
        "p",
        hlc(20, 0, 2),
        false,
    ));
    assert!(c.validate().is_member(&phone.endpoint_id()));

    // A non-admin cannot revoke anyone.
    let bad = issue_device_cert(
        &mallory,
        LIB,
        phone.endpoint_id(),
        Role::Companion,
        "p",
        hlc(30, 0, 4),
        true,
    );
    assert_eq!(
        c.check_device_cert(&bad),
        Err(IdentityError::SignerNotAdmin)
    );
    c.add_device(bad);
    assert!(
        c.validate().is_member(&phone.endpoint_id()),
        "the bogus revocation changed nothing"
    );

    // An admin can; so can the root.
    let by_admin = issue_device_cert(
        &admin,
        LIB,
        phone.endpoint_id(),
        Role::Companion,
        "p",
        hlc(40, 0, 2),
        true,
    );
    assert_eq!(c.check_device_cert(&by_admin), Ok(()));
    let mut with_admin_revocation = chain_clone(&c);
    with_admin_revocation.add_device(by_admin);
    let v = with_admin_revocation.validate();
    assert!(
        !v.is_member(&phone.endpoint_id()),
        "revoked devices are no longer members"
    );
    assert!(
        v.devices
            .iter()
            .any(|d| d.device == phone.endpoint_id() && d.revoked),
        "but the revocation is recorded"
    );

    let mut with_root_revocation = chain_clone(&c);
    with_root_revocation.add_device(issue_device_cert(
        &root,
        LIB,
        phone.endpoint_id(),
        Role::Companion,
        "p",
        hlc(40, 0, 1),
        true,
    ));
    assert!(!with_root_revocation
        .validate()
        .is_member(&phone.endpoint_id()));
}

fn chain_clone(c: &TrustChain) -> TrustChain {
    c.clone()
}

#[test]
fn a_revoked_admin_loses_the_right_to_certify_from_then_on() {
    let (root, admin, phone, laptop) = (key(1), key(2), key(3), key(4));
    let mut c = chain(&root);
    c.add_admin(issue_admin_cert(
        &root,
        LIB,
        admin.endpoint_id(),
        hlc(10, 0, 1),
        None,
    ));
    // Certified while still an admin.
    let early = issue_device_cert(
        &admin,
        LIB,
        phone.endpoint_id(),
        Role::Companion,
        "p",
        hlc(20, 0, 2),
        false,
    );
    c.add_device(early.clone());
    // What the root re-signs when it removes the admin (A-085): its certs that are current.
    assert_eq!(c.certs_signed_by(&admin.endpoint_id()), vec![early.clone()]);
    // The root revokes the admin device at 30.
    c.add_device(issue_device_cert(
        &root,
        LIB,
        admin.endpoint_id(),
        Role::Keeper,
        "old admin",
        hlc(30, 0, 1),
        true,
    ));
    // Anything it signs afterwards is invalid.
    let late = issue_device_cert(
        &admin,
        LIB,
        laptop.endpoint_id(),
        Role::Keeper,
        "l",
        hlc(40, 0, 2),
        false,
    );
    assert_eq!(
        c.check_device_cert(&late),
        Err(IdentityError::SignerNotAdmin)
    );
    c.add_device(late);
    let v = c.validate();
    // Its earlier certs no longer count either: a removed admin could have backdated them.
    assert!(!v.is_member(&phone.endpoint_id()), "not re-signed yet");
    assert!(!v.is_member(&laptop.endpoint_id()));
    assert!(!v.is_member(&admin.endpoint_id()));
    assert!(
        v.admins.iter().all(|a| a.admin != admin.endpoint_id()),
        "a revoked device is no admin"
    );

    // The root re-signs the phone's cert in the same step: it stays.
    c.add_device(issue_device_cert(
        &root,
        LIB,
        phone.endpoint_id(),
        early.role,
        &early.name,
        hlc(30, 1, 1),
        false,
    ));
    assert!(c.validate().is_member(&phone.endpoint_id()));
}

/// A-085: a stolen admin device picks its certs' dates, so the rules that stop it must not
/// depend on dates.
#[test]
fn a_stolen_admin_cannot_backdate_its_way_back() {
    let (root, real, thief, phone, extra) = (key(1), key(2), key(3), key(4), key(5));
    let mut c = chain(&root);
    for (k, t) in [(&real, 10), (&thief, 11)] {
        c.add_admin(issue_admin_cert(
            &root,
            LIB,
            k.endpoint_id(),
            hlc(t, 0, 1),
            None,
        ));
        c.add_device(issue_device_cert(
            &root,
            LIB,
            k.endpoint_id(),
            Role::Companion,
            "admin",
            hlc(t, 1, 1),
            false,
        ));
    }
    c.add_device(issue_device_cert(
        &real,
        LIB,
        phone.endpoint_id(),
        Role::Companion,
        "p",
        hlc(15, 0, 2),
        false,
    ));

    // An admin cannot remove another admin device, whatever date it writes.
    let coup = issue_device_cert(
        &thief,
        LIB,
        real.endpoint_id(),
        Role::Companion,
        "admin",
        hlc(12, 0, 3),
        true,
    );
    assert_eq!(c.check_device_cert(&coup), Err(IdentityError::RootRequired));
    c.add_device(coup);
    assert!(c.validate().is_member(&real.endpoint_id()));
    // An admin can still remove an ordinary device.
    let lost = issue_device_cert(
        &real,
        LIB,
        phone.endpoint_id(),
        Role::Companion,
        "p",
        hlc(16, 0, 2),
        true,
    );
    assert_eq!(c.check_device_cert(&lost), Ok(()));

    // The root removes the thief at 50.
    c.add_device(issue_device_cert(
        &root,
        LIB,
        thief.endpoint_id(),
        Role::Companion,
        "admin",
        hlc(50, 0, 1),
        true,
    ));
    // A cert dated before that still does not count...
    let backdated = issue_device_cert(
        &thief,
        LIB,
        extra.endpoint_id(),
        Role::Keeper,
        "x",
        hlc(20, 0, 3),
        false,
    );
    c.add_device(backdated);
    // ...and no admin can bring the thief back; only the root could.
    let revived = issue_device_cert(
        &real,
        LIB,
        thief.endpoint_id(),
        Role::Companion,
        "admin",
        hlc(60, 0, 2),
        false,
    );
    assert_eq!(
        c.check_device_cert(&revived),
        Err(IdentityError::RootRequired)
    );
    c.add_device(revived);
    let v = c.validate();
    assert!(!v.is_member(&extra.endpoint_id()), "backdated cert ignored");
    assert!(!v.is_member(&thief.endpoint_id()), "stays removed");
    assert!(v.is_member(&real.endpoint_id()));
    assert!(v.is_member(&phone.endpoint_id()));
}

#[test]
fn validation_does_not_depend_on_arrival_order() {
    let (root, admin, phone) = (key(1), key(2), key(3));
    let a = issue_admin_cert(&root, LIB, admin.endpoint_id(), hlc(10, 0, 1), None);
    let d1 = issue_device_cert(
        &admin,
        LIB,
        phone.endpoint_id(),
        Role::Companion,
        "p",
        hlc(20, 0, 2),
        false,
    );
    let d2 = issue_device_cert(
        &admin,
        LIB,
        phone.endpoint_id(),
        Role::Companion,
        "renamed",
        hlc(25, 0, 2),
        false,
    );
    let d3 = issue_device_cert(
        &root,
        LIB,
        phone.endpoint_id(),
        Role::Companion,
        "renamed",
        hlc(30, 0, 1),
        true,
    );
    let build = |order: &[usize]| {
        let mut c = chain(&root);
        for &i in order {
            match i {
                0 => c.add_admin(a.clone()),
                1 => c.add_device(d1.clone()),
                2 => c.add_device(d2.clone()),
                _ => c.add_device(d3.clone()),
            }
        }
        c.validate()
    };
    let reference = build(&[0, 1, 2, 3]);
    for order in [[3, 2, 1, 0], [2, 0, 3, 1], [1, 3, 0, 2]] {
        assert_eq!(build(&order), reference, "order {order:?}");
    }
    assert!(!reference.is_member(&phone.endpoint_id()));
    assert_eq!(reference.devices.len(), 1, "latest valid cert per device");
    assert!(reference.devices[0].revoked);
}
