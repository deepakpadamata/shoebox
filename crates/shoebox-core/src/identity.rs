//! Library identity (M2-01): who may add devices.
//!
//! Trust flows from the library's **root key** (held only in the recovery kit): the root signs
//! [`AdminCert`]s delegating to admin devices, and the root or an admin signs a [`DeviceCert`] per
//! device (revocation is a newer cert with `revoked = true`). [`TrustChain::validate`] replays
//! the certs in HLC order, so the outcome never depends on the order they arrived in.

use crate::ids::{EndpointId, Timestamp};
use crate::model::{AdminCert, DeviceCert, Role};
use crate::store::{LibraryId, RootPublicKey};
use crate::Hlc;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IdentityError {
    #[error("signature does not verify")]
    BadSignature,
    #[error("signer was not an admin of this library at that time")]
    SignerNotAdmin,
    #[error("the signer's admin delegation had expired")]
    Expired,
}

/// An ed25519 key pair (a device key, or the library root key).
#[derive(Clone)]
pub struct KeyPair(SigningKey);

impl KeyPair {
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self(SigningKey::from_bytes(&seed))
    }
    /// A fresh key from the operating system's random source.
    pub fn generate() -> Self {
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).expect("operating system randomness");
        Self::from_seed(seed)
    }
    pub fn seed(&self) -> [u8; 32] {
        self.0.to_bytes()
    }
    pub fn public_bytes(&self) -> [u8; 32] {
        self.0.verifying_key().to_bytes()
    }
    pub fn endpoint_id(&self) -> EndpointId {
        EndpointId(self.public_bytes())
    }
    pub fn root_public(&self) -> RootPublicKey {
        RootPublicKey(self.public_bytes())
    }
    /// Alias of [`KeyPair::root_public`] for call sites that hold the root key.
    pub fn public(&self) -> RootPublicKey {
        self.root_public()
    }
    pub fn sign(&self, msg: &[u8]) -> Vec<u8> {
        self.0.sign(msg).to_bytes().to_vec()
    }
}

impl std::fmt::Debug for KeyPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "KeyPair({})", hex::encode(self.public_bytes()))
    }
}

/// Strict ed25519 verification; malformed keys or signatures simply fail.
pub fn verify_signature(public: &[u8; 32], msg: &[u8], sig: &[u8]) -> bool {
    let Ok(sig): Result<[u8; 64], _> = sig.try_into() else {
        return false;
    };
    let Ok(key) = VerifyingKey::from_bytes(public) else {
        return false;
    };
    key.verify_strict(msg, &Signature::from_bytes(&sig)).is_ok()
}

fn signing_bytes<T: Serialize>(domain: &str, library: LibraryId, body: &T) -> Vec<u8> {
    let mut out = domain.as_bytes().to_vec();
    out.push(0);
    out.extend_from_slice(&library.0);
    out.extend(postcard::to_stdvec(body).expect("cert fields encode"));
    out
}

fn admin_bytes(library: LibraryId, c: &AdminCert) -> Vec<u8> {
    signing_bytes(
        "shoebox/admin-cert/v1",
        library,
        &(&c.admin, &c.hlc, &c.expires_at),
    )
}

fn device_bytes(library: LibraryId, c: &DeviceCert) -> Vec<u8> {
    signing_bytes(
        "shoebox/device-cert/v1",
        library,
        &(&c.device, &c.role, &c.name, &c.added_by, &c.hlc, &c.revoked),
    )
}

/// Root-signed delegation of the right to certify devices.
pub fn issue_admin_cert(
    root: &KeyPair,
    library: LibraryId,
    admin: EndpointId,
    hlc: Hlc,
    expires_at: Option<Timestamp>,
) -> AdminCert {
    let mut cert = AdminCert {
        admin,
        hlc,
        expires_at,
        signature: vec![],
    };
    cert.signature = root.sign(&admin_bytes(library, &cert));
    cert
}

/// A device certificate (or, with `revoked`, its revocation) signed by `signer` — the root key
/// or an admin device key. `added_by` is set to the signer's id.
pub fn issue_device_cert(
    signer: &KeyPair,
    library: LibraryId,
    device: EndpointId,
    role: Role,
    name: &str,
    hlc: Hlc,
    revoked: bool,
) -> DeviceCert {
    let mut cert = DeviceCert {
        device,
        role,
        name: name.to_string(),
        added_by: signer.endpoint_id(),
        hlc,
        revoked,
        signature: vec![],
    };
    cert.signature = signer.sign(&device_bytes(library, &cert));
    cert
}

/// The accepted certs of a library, after validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidChain {
    pub admins: Vec<AdminCert>,
    /// The latest accepted cert per device (revocations included), sorted by device id.
    pub devices: Vec<DeviceCert>,
}

impl ValidChain {
    /// Certified and not revoked.
    pub fn is_member(&self, device: &EndpointId) -> bool {
        self.devices
            .iter()
            .any(|d| d.device == *device && !d.revoked)
    }
}

/// All certs seen so far for one library, plus the root key they must chain to.
#[derive(Debug, Clone)]
pub struct TrustChain {
    root: RootPublicKey,
    library: LibraryId,
    admins: Vec<AdminCert>,
    devices: Vec<DeviceCert>,
}

impl TrustChain {
    pub fn new(root: RootPublicKey, library: LibraryId) -> Self {
        Self {
            root,
            library,
            admins: vec![],
            devices: vec![],
        }
    }

    /// Admin certs must be signed by the root.
    pub fn check_admin(&self, cert: &AdminCert) -> Result<(), IdentityError> {
        if verify_signature(
            &self.root.0,
            &admin_bytes(self.library, cert),
            &cert.signature,
        ) {
            Ok(())
        } else {
            Err(IdentityError::BadSignature)
        }
    }

    /// Only the signature (against the cert's own `added_by` key), not the signer's authority.
    pub fn signature_ok(&self, cert: &DeviceCert) -> bool {
        verify_signature(
            &cert.added_by.0,
            &device_bytes(self.library, cert),
            &cert.signature,
        )
    }

    pub fn add_admin(&mut self, cert: AdminCert) {
        if !self.admins.contains(&cert) {
            self.admins.push(cert);
        }
    }

    pub fn add_device(&mut self, cert: DeviceCert) {
        if !self.devices.contains(&cert) {
            self.devices.push(cert);
        }
    }

    /// Would `cert` be accepted given the certs stored now (those older than it)?
    pub fn check_device_cert(&self, cert: &DeviceCert) -> Result<(), IdentityError> {
        let accepted = self.replay(Some(&cert.hlc));
        self.authorise(cert, &accepted)
    }

    /// Replay every cert in HLC order and keep the ones that were valid when issued.
    pub fn validate(&self) -> ValidChain {
        let accepted = self.replay(None);
        let mut admins: Vec<AdminCert> = self
            .admins
            .iter()
            .filter(|a| self.check_admin(a).is_ok())
            .cloned()
            .collect();
        admins.sort_by_key(|a| (a.admin, a.hlc));
        ValidChain {
            admins,
            devices: accepted.into_values().collect(),
        }
    }

    fn replay(&self, before: Option<&Hlc>) -> BTreeMap<EndpointId, DeviceCert> {
        let mut certs: Vec<&DeviceCert> = self
            .devices
            .iter()
            .filter(|c| before.is_none_or(|b| c.hlc < *b))
            .collect();
        certs.sort_by(|a, b| {
            (a.hlc, a.device, a.revoked, &a.signature).cmp(&(
                b.hlc,
                b.device,
                b.revoked,
                &b.signature,
            ))
        });
        let mut accepted: BTreeMap<EndpointId, DeviceCert> = BTreeMap::new();
        for cert in certs {
            if self.authorise(cert, &accepted).is_ok() {
                accepted.insert(cert.device, cert.clone());
            }
        }
        accepted
    }

    fn authorise(
        &self,
        cert: &DeviceCert,
        accepted: &BTreeMap<EndpointId, DeviceCert>,
    ) -> Result<(), IdentityError> {
        let msg = device_bytes(self.library, cert);
        if !verify_signature(&cert.added_by.0, &msg, &cert.signature) {
            return Err(IdentityError::BadSignature);
        }
        if cert.added_by.0 == self.root.0 {
            return Ok(());
        }
        // A delegated signer must hold a root-signed, currently valid admin cert and must not
        // have been revoked before this cert was issued.
        let now = cert.hlc.physical_ms as i64;
        let mut expired = false;
        let mut delegated = false;
        for a in self
            .admins
            .iter()
            .filter(|a| a.admin == cert.added_by && self.check_admin(a).is_ok())
            .filter(|a| (a.hlc.physical_ms as i64) <= now)
        {
            match a.expires_at {
                Some(Timestamp(t)) if now >= t => expired = true,
                _ => delegated = true,
            }
        }
        let revoked = accepted.get(&cert.added_by).is_some_and(|c| c.revoked);
        if delegated && !revoked {
            Ok(())
        } else if expired && !revoked {
            Err(IdentityError::Expired)
        } else {
            Err(IdentityError::SignerNotAdmin)
        }
    }
}

/// Announces that a library moved to a new epoch after `revocation` (M2-03b). Anyone holding
/// the library's root key can check it: the admin that signed it must hold a root-signed admin
/// cert, and the revocation it carries must be valid, so a member that was offline during the
/// revocation can verify both the revocation and the move from this one message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct RotationNotice {
    pub old: LibraryId,
    pub new_cap: crate::store::LibraryCapability,
    pub revocation: DeviceCert,
    pub admin: AdminCert,
    pub signature: Vec<u8>,
}

fn rotation_bytes(identity: LibraryId, n: &RotationNotice) -> Vec<u8> {
    signing_bytes(
        "shoebox/rotation/v1",
        identity,
        &(&n.old, &n.new_cap, &n.revocation, &n.admin),
    )
}

/// Sign a rotation notice with the admin device key (`admin` must be that device's admin cert).
pub fn issue_rotation_notice(
    admin_key: &KeyPair,
    identity: LibraryId,
    old: LibraryId,
    new_cap: crate::store::LibraryCapability,
    revocation: DeviceCert,
    admin: AdminCert,
) -> RotationNotice {
    let mut n = RotationNotice {
        old,
        new_cap,
        revocation,
        admin,
        signature: vec![],
    };
    n.signature = admin_key.sign(&rotation_bytes(identity, &n));
    n
}

impl RotationNotice {
    /// Check the whole notice against the library's root key and identity id.
    pub fn verify(&self, root: &RootPublicKey, identity: LibraryId) -> Result<(), IdentityError> {
        let mut chain = TrustChain::new(*root, identity);
        chain.check_admin(&self.admin)?;
        chain.add_admin(self.admin.clone());
        if !verify_signature(
            &self.admin.admin.0,
            &rotation_bytes(identity, self),
            &self.signature,
        ) {
            return Err(IdentityError::BadSignature);
        }
        if !self.revocation.revoked
            || self.new_cap.root != *root
            || self.new_cap.identity_id() != identity
        {
            return Err(IdentityError::BadSignature);
        }
        chain.check_device_cert(&self.revocation)
    }
}
