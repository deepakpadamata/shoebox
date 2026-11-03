//! Signed requests to Shoebox Cloud (W1-04). No accounts: a request is signed by a device key, and carries the
//! certificates that show the device belongs to the library (the chain of M2-01). The service keeps the library's
//! root public key from its first request and checks every later chain against it.
//!
//! The signature covers the method, the path and query, the time and the BLAKE3 hash of the body, under the
//! domain `shoebox/cloud-request/v1`, so it cannot be replayed on another request or as another kind of
//! signature, and a captured request goes stale after the service's clock-skew window.

use crate::identity::{verify_signature, KeyPair};
use crate::model::{AdminCert, DeviceCert};
use crate::store::{LibraryId, RootPublicKey};
use serde::{Deserialize, Serialize};

const DOMAIN: &str = "shoebox/cloud-request/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    #[error("the credentials are not readable")]
    Malformed,
    #[error("the request is not signed by the device named in its certificate")]
    BadSignature,
    #[error("the request's time is too far from the service's clock")]
    Stale,
}

/// What a request carries to prove who sent it.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct CloudAuth {
    pub library: LibraryId,
    pub root: RootPublicKey,
    /// The admin delegation of the device's certifier, when a device (not the root) certified it.
    pub admin: Option<AdminCert>,
    pub device: DeviceCert,
    pub time_ms: i64,
    pub signature: Vec<u8>,
}

fn signing_bytes(
    library: LibraryId,
    method: &str,
    path: &str,
    time_ms: i64,
    body_hash: &[u8; 32],
) -> Vec<u8> {
    let mut out = DOMAIN.as_bytes().to_vec();
    out.push(0);
    out.extend_from_slice(&library.0);
    out.extend_from_slice(method.as_bytes());
    out.push(0);
    out.extend_from_slice(path.as_bytes());
    out.push(0);
    out.extend_from_slice(&time_ms.to_le_bytes());
    out.extend_from_slice(body_hash);
    out
}

impl CloudAuth {
    /// Sign a request. `path` is the path and query exactly as sent; `body` the bytes of the body (empty for none).
    #[allow(clippy::too_many_arguments)]
    pub fn sign(
        key: &KeyPair,
        library: LibraryId,
        root: RootPublicKey,
        admin: Option<AdminCert>,
        device: DeviceCert,
        method: &str,
        path: &str,
        now_ms: i64,
        body: &[u8],
    ) -> Self {
        let hash = *blake3::hash(body).as_bytes();
        Self::sign_hashed(
            key, library, root, admin, device, method, path, now_ms, &hash,
        )
    }

    /// [`CloudAuth::sign`] when the hash of the body is already known (a blob upload: it is the blob's id), so the
    /// service can check the signature before it reads the body.
    #[allow(clippy::too_many_arguments)]
    pub fn sign_hashed(
        key: &KeyPair,
        library: LibraryId,
        root: RootPublicKey,
        admin: Option<AdminCert>,
        device: DeviceCert,
        method: &str,
        path: &str,
        now_ms: i64,
        body_hash: &[u8; 32],
    ) -> Self {
        let signature = key.sign(&signing_bytes(library, method, path, now_ms, body_hash));
        Self {
            library,
            root,
            admin,
            device,
            time_ms: now_ms,
            signature,
        }
    }

    /// Check the signature and the time. Whether the device is a current member of the library is the
    /// service's own check against the chain.
    pub fn verify(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
        now_ms: i64,
        skew_ms: i64,
    ) -> Result<(), AuthError> {
        let hash = *blake3::hash(body).as_bytes();
        self.verify_hashed(method, path, &hash, now_ms, skew_ms)
    }

    /// [`CloudAuth::verify`] with the body's hash instead of the body.
    pub fn verify_hashed(
        &self,
        method: &str,
        path: &str,
        body_hash: &[u8; 32],
        now_ms: i64,
        skew_ms: i64,
    ) -> Result<(), AuthError> {
        if (now_ms - self.time_ms).abs() > skew_ms {
            return Err(AuthError::Stale);
        }
        let msg = signing_bytes(self.library, method, path, self.time_ms, body_hash);
        if verify_signature(&self.device.device.0, &msg, &self.signature) {
            Ok(())
        } else {
            Err(AuthError::BadSignature)
        }
    }

    /// The text put in the `Authorization` header (after `Shoebox `): hex of the postcard encoding.
    pub fn to_header(&self) -> String {
        hex::encode(postcard::to_stdvec(self).expect("credentials encode"))
    }

    pub fn from_header(text: &str) -> Result<Self, AuthError> {
        let bytes = hex::decode(text.trim()).map_err(|_| AuthError::Malformed)?;
        postcard::from_bytes(&bytes).map_err(|_| AuthError::Malformed)
    }
}

/// Certificates a member sends the service so it learns of revocations (`POST /v1/library/certs`).
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct CertBundle {
    pub admins: Vec<AdminCert>,
    pub devices: Vec<DeviceCert>,
}
