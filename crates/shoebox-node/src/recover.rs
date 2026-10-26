//! Getting back into a library with the recovery phrase when this device has nothing (A-060).
//!
//! The device dials a surviving member (named by a [`PeerLink`]) on `shoebox/recover/0`:
//!
//! 1. device → `Hello`
//! 2. member → `Challenge{nonce, library identity}`
//! 3. device → `Proof{root public key, signature over the nonce, its own id and the identity,
//!    plus an admin cert and a device cert for itself signed with the root key}`
//! 4. member checks the key is the library's root key, the signature, and both certificates; it
//!    records the certificates (so the device is a listed admin member at once) and replies
//!    `Granted{library capability, its own address}`.
//!
//! Only the holder of the root key can pass step 4, so nothing secret is exchanged in the clear
//! and a wrong phrase changes nothing on the member. Failed attempts are rate limited.
//!
//! A phrase can leak, so before step 4's records are written a member with a screen asks its
//! person to approve ([`LibraryNode::set_recovery_approval`]); a headless member (a keeper)
//! cannot ask and lets it through. Either way [`NodeEvent::RecoveryJoined`] reports it, and every
//! device alerts on the new admin ([`NodeEvent::AdminAdded`]).

use crate::addr::{from_endpoint_addr, from_iroh_id, to_endpoint_addr};
use crate::library_node::{Inner, LibraryNode};
use crate::node::{Node, NodeEvent, RECOVER_ALPN};
use anyhow::{anyhow, bail, ensure, Context, Result};
use futures::future::BoxFuture;
use iroh::endpoint::Connection;
use serde::{Deserialize, Serialize};
use shoebox_core::identity::{
    issue_admin_cert, issue_device_cert, verify_signature, KeyPair, TrustChain,
};
use shoebox_core::ids::EndpointId;
use shoebox_core::model::{AdminCert, DeviceCert, Role};
use shoebox_core::recovery::RecoveryKit;
use shoebox_core::store::{
    LibraryCapability, LibraryId, LibraryStore, PeerAddr, RootPublicKey, SignedDeviceChange,
};
use shoebox_core::Hlc;
use std::fmt;
use std::str::FromStr;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

const PREFIX: &str = "shoebox-peer1";
const MAX_FAILED_PER_MINUTE: usize = 10;
const MAX_MESSAGE: usize = 1 << 20;
const STEP_TIMEOUT: Duration = Duration::from_secs(20);
/// How long a member waits for its person to approve a recovery.
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);
/// The whole exchange, as seen from either side: two steps plus the approval.
const RECOVERY_TIMEOUT: Duration = Duration::from_secs(20 * 3 + 120);

/// Someone is rejoining the library with the recovery phrase through this device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryRequest {
    pub device: EndpointId,
    /// The name the device asks for (chosen by whoever holds the phrase; at most 64 characters).
    pub name: String,
    pub role: Role,
}

/// Asks this device's person whether to let a recovery through: `true` = allow.
pub type RecoveryDecideFn = Arc<dyn Fn(RecoveryRequest) -> BoxFuture<'static, bool> + Send + Sync>;

/// How to reach one device: what a surviving device shows so a recovering one can find it.
/// Contains no secret.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerLink(pub PeerAddr);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("not a Shoebox device link (expected `{PREFIX}…`, or it was damaged)")]
pub struct PeerLinkError;

fn checksum(body: &[u8]) -> [u8; 4] {
    let mut h = blake3::Hasher::new();
    h.update(PREFIX.as_bytes());
    h.update(body);
    h.finalize().as_bytes()[..4].try_into().expect("4 bytes")
}

impl fmt::Display for PeerLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut body = postcard::to_stdvec(self).expect("link encodes");
        let sum = checksum(&body);
        body.extend(sum);
        write!(f, "{PREFIX}{}", hex::encode(body))
    }
}

impl FromStr for PeerLink {
    type Err = PeerLinkError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let rest = s.trim().strip_prefix(PREFIX).ok_or(PeerLinkError)?;
        let mut bytes = hex::decode(rest).map_err(|_| PeerLinkError)?;
        if bytes.len() < 4 {
            return Err(PeerLinkError);
        }
        let sum = bytes.split_off(bytes.len() - 4);
        if sum != checksum(&bytes) {
            return Err(PeerLinkError);
        }
        postcard::from_bytes(&bytes).map_err(|_| PeerLinkError)
    }
}

/// What a recovering device receives.
#[derive(Clone, Debug)]
pub struct RecoverGrant {
    pub cap: LibraryCapability,
    pub role: Role,
    /// Devices to sync with first.
    pub peers: Vec<PeerAddr>,
}

#[derive(Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
enum ToMember {
    Hello,
    Proof {
        root: RootPublicKey,
        signature: Vec<u8>,
        admin: AdminCert,
        device: DeviceCert,
    },
}

#[derive(Serialize, Deserialize)]
enum ToDevice {
    Challenge {
        nonce: [u8; 16],
        identity: LibraryId,
    },
    Granted {
        cap: LibraryCapability,
        peers: Vec<PeerAddr>,
    },
    Refused(String),
}

fn proof_message(nonce: &[u8; 16], device: &EndpointId, identity: LibraryId) -> Vec<u8> {
    let mut m = b"shoebox/recover/v1\0".to_vec();
    m.extend_from_slice(nonce);
    m.extend_from_slice(&device.0);
    m.extend_from_slice(&identity.0);
    m
}

async fn write_msg<T: Serialize>(send: &mut iroh::endpoint::SendStream, msg: &T) -> Result<()> {
    let body = postcard::to_stdvec(msg)?;
    send.write_all(&(body.len() as u32).to_be_bytes()).await?;
    send.write_all(&body).await?;
    Ok(())
}

async fn read_msg<T: for<'de> Deserialize<'de>>(
    recv: &mut iroh::endpoint::RecvStream,
) -> Result<T> {
    let mut len = [0u8; 4];
    recv.read_exact(&mut len).await.context("reading message")?;
    let len = u32::from_be_bytes(len) as usize;
    ensure!(len <= MAX_MESSAGE, "message too large");
    let mut body = vec![0u8; len];
    recv.read_exact(&mut body)
        .await
        .context("reading message")?;
    Ok(postcard::from_bytes(&body)?)
}

struct Responder<L: LibraryStore> {
    inner: Weak<Inner<L>>,
    failed: Arc<Mutex<crate::limit::FailureLimit>>,
}

impl<L: LibraryStore> crate::ext::ConnHandler for Responder<L> {
    fn handle(&self, conn: Connection) -> BoxFuture<'static, Result<()>> {
        let inner = self.inner.upgrade();
        let failed = self.failed.clone();
        Box::pin(async move {
            let ln = LibraryNode {
                inner: inner.ok_or_else(|| anyhow!("library closed"))?,
            };
            let (mut send, mut recv) = conn.accept_bi().await?;
            let result = tokio::time::timeout(
                RECOVERY_TIMEOUT,
                serve(&ln, &failed, &conn, &mut send, &mut recv),
            )
            .await
            .map_err(|_| anyhow!("timed out"))
            .and_then(|r| r);
            if let Err(e) = &result {
                let _ = write_msg(&mut send, &ToDevice::Refused(format!("{e:#}"))).await;
            }
            let _ = send.finish();
            let _ = tokio::time::timeout(Duration::from_secs(2), conn.closed()).await;
            result
        })
    }
}

async fn serve<L: LibraryStore>(
    ln: &LibraryNode<L>,
    failed: &Mutex<crate::limit::FailureLimit>,
    conn: &Connection,
    send: &mut iroh::endpoint::SendStream,
    recv: &mut iroh::endpoint::RecvStream,
) -> Result<()> {
    let now = ln.now_ms();
    let peer = conn.remote_id();
    ensure!(
        failed
            .lock()
            .unwrap()
            .allows(&peer, now, MAX_FAILED_PER_MINUTE),
        "too many failed attempts, try again later"
    );
    let fail = |why: &str| -> anyhow::Error {
        failed.lock().unwrap().record(peer, now);
        anyhow!("{why}")
    };
    let ToMember::Hello = step(read_msg(recv)).await? else {
        return Err(fail("expected hello"));
    };
    let cap = ln.capability()?;
    let identity = cap.identity_id();
    let device = from_iroh_id(&conn.remote_id());
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce).expect("operating system randomness");
    write_msg(send, &ToDevice::Challenge { nonce, identity }).await?;
    let ToMember::Proof {
        root,
        signature,
        admin,
        device: cert,
    } = step(read_msg(recv)).await?
    else {
        return Err(fail("expected proof"));
    };
    if root != cap.root
        || !verify_signature(
            &root.0,
            &proof_message(&nonce, &device, identity),
            &signature,
        )
    {
        return Err(fail("recovery proof rejected"));
    }
    let mut chain = TrustChain::new(cap.root, identity);
    if chain.check_admin(&admin).is_err()
        || admin.admin != device
        || {
            chain.add_admin(admin.clone());
            chain.check_device_cert(&cert).is_err()
        }
        || cert.device != device
        || cert.revoked
    {
        return Err(fail("recovery proof rejected"));
    }
    // The proof holds, but the phrase may have leaked: ask, when there is someone to ask.
    let name: String = cert.name.chars().take(64).collect();
    let decide = ln.inner.recovery_approval.lock().unwrap().clone();
    if let Some(decide) = decide {
        let request = RecoveryRequest {
            device,
            name: name.clone(),
            role: cert.role,
        };
        let allowed = tokio::time::timeout(APPROVAL_TIMEOUT, decide(request))
            .await
            .unwrap_or(false);
        ensure!(allowed, "the recovery was not approved on that device");
    }
    // List the device (as an admin) so it can connect and sync.
    ln.store()
        .apply_device_change(SignedDeviceChange::Admin(admin))
        .await?;
    ln.store()
        .apply_device_change(SignedDeviceChange::Device(cert))
        .await?;
    write_msg(
        send,
        &ToDevice::Granted {
            cap,
            peers: vec![from_endpoint_addr(&ln.node().addr())],
        },
    )
    .await?;
    tracing::warn!("{name} rejoined the library with the recovery phrase through this device");
    ln.node().emit(NodeEvent::RecoveryJoined { device, name });
    Ok(())
}

/// One message exchange of the handshake, bounded so a silent peer cannot hold it open.
async fn step<T>(read: impl std::future::Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(STEP_TIMEOUT, read)
        .await
        .map_err(|_| anyhow!("timed out"))?
}

impl<L: LibraryStore> LibraryNode<L> {
    pub(crate) fn start_recovery_responder(&self) {
        let responder: Arc<dyn crate::ext::ConnHandler> = Arc::new(Responder {
            inner: Arc::downgrade(&self.inner),
            failed: Arc::default(),
        });
        *self.node().recover_slot().lock().unwrap() = Some(responder);
    }

    /// Ask before letting a recovery through this device (`None`: let it through, as a device
    /// without a screen must). Answer within two minutes or the recovery is refused.
    pub fn set_recovery_approval(&self, decide: Option<RecoveryDecideFn>) {
        *self.inner.recovery_approval.lock().unwrap() = decide;
    }

    /// How a recovering device can reach this one.
    pub fn peer_link(&self) -> PeerLink {
        PeerLink(from_endpoint_addr(&self.node().addr()))
    }
}

impl Node {
    /// Rejoin a library with its recovery phrase: dial the surviving device `from`, prove
    /// possession of the root key, and receive the library capability. This device becomes a
    /// listed admin member with `role` and `name`; the caller then opens the library with
    /// `grant.cap` and syncs. `now_ms` stamps the new certificates.
    pub async fn recover(
        &self,
        from: &PeerLink,
        phrase: &str,
        role: Role,
        name: &str,
        now_ms: u64,
    ) -> Result<RecoverGrant> {
        let root: KeyPair = RecoveryKit::root_from_phrase(phrase)?;
        let me = from_iroh_id(&self.id());
        let addr = to_endpoint_addr(&from.0).map_err(|e| anyhow!("{e}"))?;
        let run = async {
            let conn = self
                .endpoint()
                .connect(addr, RECOVER_ALPN)
                .await
                .context("could not reach the device")?;
            let (mut send, mut recv) = conn.open_bi().await?;
            write_msg(&mut send, &ToMember::Hello).await?;
            let (nonce, identity) = match read_msg::<ToDevice>(&mut recv).await? {
                ToDevice::Challenge { nonce, identity } => (nonce, identity),
                ToDevice::Refused(why) => bail!("{why}"),
                ToDevice::Granted { .. } => bail!("unexpected grant"),
            };
            let hlc = Hlc {
                physical_ms: now_ms,
                counter: 0,
                device: me,
            };
            write_msg(
                &mut send,
                &ToMember::Proof {
                    root: root.root_public(),
                    signature: root.sign(&proof_message(&nonce, &me, identity)),
                    admin: issue_admin_cert(&root, identity, me, hlc, None),
                    device: issue_device_cert(&root, identity, me, role, name, hlc, false),
                },
            )
            .await?;
            send.finish()?;
            let grant = match read_msg::<ToDevice>(&mut recv).await? {
                ToDevice::Granted { cap, peers } => RecoverGrant { cap, role, peers },
                ToDevice::Refused(why) => bail!("{why}"),
                ToDevice::Challenge { .. } => bail!("unexpected challenge"),
            };
            conn.close(0u32.into(), b"done");
            Ok::<RecoverGrant, anyhow::Error>(grant)
        };
        let grant = tokio::time::timeout(RECOVERY_TIMEOUT, run)
            .await
            .map_err(|_| anyhow!("timed out"))??;
        ensure!(
            grant.cap.root == root.root_public(),
            "the library belongs to another root key"
        );
        Ok(grant)
    }
}
