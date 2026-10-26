//! Device pairing over `shoebox/pair/0` (M2-04).
//!
//! An admin device (the *inviter*) creates a [`PairTicket`] (shown as a QR code or text). The new
//! device (the *joiner*) dials it and both sides run:
//!
//! 1. joiner → `Hello{ticket id, HMAC proof of the secret, nonce, name}`
//! 2. inviter checks the ticket (unexpired, unused, not rate-limited) and consumes it, then
//!    answers `Challenge{nonce, role}`
//! 3. both derive the same six-digit code from the secret, both ids and both nonces; each person
//!    compares the codes and confirms (the joiner sends `Confirm`)
//! 4. only if both confirmed, the inviter signs a [`DeviceCert`], applies it to its replica and
//!    sends `Granted{cert, its admin cert, library capability, peers}`; otherwise `Refused`.
//!
//! The ticket secret authenticates the exchange; the code protects against someone who saw the
//! ticket and raced the real device. Nothing is written on either side until step 4.

use crate::addr::{from_endpoint_addr, from_iroh_id, to_endpoint_addr};
use crate::library_node::{Inner, LibraryNode};
use crate::node::{Node, NodeEvent, PAIR_ALPN};
use anyhow::{anyhow, bail, ensure, Context, Result};
use futures::future::BoxFuture;
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use serde::{Deserialize, Serialize};
use shoebox_core::identity::{issue_device_cert, TrustChain};
use shoebox_core::ids::{EndpointId, Timestamp};
use shoebox_core::model::{AdminCert, DeviceCert, Role};
use shoebox_core::store::{LibraryCapability, LibraryStore, PeerAddr, SignedDeviceChange};
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

const PREFIX: &str = "shoebox-pair1";
/// Failed proofs after which a ticket is burned.
const MAX_FAILED_PROOFS: u32 = 5;
/// Failed proofs across all tickets within a minute before pairing refuses everything.
const MAX_FAILED_PER_MINUTE: usize = 30;
const HELLO_TIMEOUT: Duration = Duration::from_secs(15);
const DECISION_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_MESSAGE: usize = 1 << 20;
const EXPIRED: &str = "ticket expired or already used";
const REJECTED: &str = "pairing proof rejected";

/// What both people compare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingCode {
    /// Six digits.
    pub sas: String,
    pub peer_name: String,
    pub peer_role: Role,
}

/// Asked once per pairing attempt: do the codes match? (`true` = confirm.)
pub type DecideFn = Arc<dyn Fn(PairingCode) -> BoxFuture<'static, bool> + Send + Sync>;

/// What the joiner shows or scans: where to dial, the secret, the expiry and the role on offer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairTicket {
    pub addr: PeerAddr,
    pub secret: [u8; 32],
    pub expires: Timestamp,
    pub role: Role,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PairTicketError {
    #[error("not a Shoebox pairing ticket (expected the `{PREFIX}` prefix)")]
    BadPrefix,
    #[error("ticket is not valid hex")]
    BadEncoding,
    #[error("ticket is truncated or was altered (checksum mismatch)")]
    BadChecksum,
    #[error("ticket contents are malformed")]
    Malformed,
}

fn checksum(body: &[u8]) -> [u8; 4] {
    let mut h = blake3::Hasher::new();
    h.update(PREFIX.as_bytes());
    h.update(body);
    h.finalize().as_bytes()[..4].try_into().expect("4 bytes")
}

impl fmt::Display for PairTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut body = postcard::to_stdvec(self).expect("ticket encodes");
        let sum = checksum(&body);
        body.extend(sum);
        write!(f, "{PREFIX}{}", hex::encode(body))
    }
}

impl FromStr for PairTicket {
    type Err = PairTicketError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let rest = s
            .trim()
            .strip_prefix(PREFIX)
            .ok_or(PairTicketError::BadPrefix)?;
        let mut bytes = hex::decode(rest).map_err(|_| PairTicketError::BadEncoding)?;
        if bytes.len() < 4 {
            return Err(PairTicketError::BadChecksum);
        }
        let sum = bytes.split_off(bytes.len() - 4);
        if sum != checksum(&bytes) {
            return Err(PairTicketError::BadChecksum);
        }
        postcard::from_bytes(&bytes).map_err(|_| PairTicketError::Malformed)
    }
}

impl PairTicket {
    fn id(&self) -> [u8; 8] {
        ticket_id(&self.secret)
    }
}

/// What the joiner receives after both sides confirmed.
#[derive(Clone, Debug)]
pub struct PairGrant {
    pub cap: LibraryCapability,
    pub cert: DeviceCert,
    pub role: Role,
    /// Devices to sync with first.
    pub peers: Vec<PeerAddr>,
}

// ---------- wire format ----------

#[derive(Serialize, Deserialize)]
enum ToInviter {
    Hello {
        ticket_id: [u8; 8],
        proof: [u8; 32],
        nonce: [u8; 16],
        name: String,
    },
    Confirm(bool),
}

#[derive(Serialize, Deserialize)]
struct Grant {
    cert: DeviceCert,
    inviter_admin: AdminCert,
    cap: LibraryCapability,
    peers: Vec<PeerAddr>,
}

#[derive(Serialize, Deserialize)]
enum ToJoiner {
    Challenge { nonce: [u8; 16], role: Role },
    Granted(Box<Grant>),
    Refused(String),
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

// ---------- crypto helpers ----------

fn random<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    getrandom::fill(&mut out).expect("operating system randomness");
    out
}

fn ticket_id(secret: &[u8; 32]) -> [u8; 8] {
    let h = blake3::keyed_hash(secret, b"shoebox/pair-ticket-id/0");
    h.as_bytes()[..8].try_into().expect("8 bytes")
}

fn proof(
    secret: &[u8; 32],
    joiner: &EndpointId,
    inviter: &EndpointId,
    nonce: &[u8; 16],
) -> blake3::Hash {
    let mut h = blake3::Hasher::new_keyed(secret);
    h.update(b"shoebox/pair-proof/0");
    h.update(&joiner.0);
    h.update(&inviter.0);
    h.update(nonce);
    h.finalize()
}

/// The six digits both people compare.
fn sas(
    secret: &[u8; 32],
    joiner: &EndpointId,
    inviter: &EndpointId,
    nj: &[u8; 16],
    ni: &[u8; 16],
) -> String {
    let mut h = blake3::Hasher::new_keyed(secret);
    h.update(b"shoebox/pair-sas/0");
    h.update(&joiner.0);
    h.update(&inviter.0);
    h.update(nj);
    h.update(ni);
    let n = u32::from_be_bytes(h.finalize().as_bytes()[..4].try_into().expect("4 bytes"));
    format!("{:06}", n % 1_000_000)
}

// ---------- the protocol handler on every node ----------

pub(crate) trait PairResponder: Send + Sync {
    fn respond(&self, conn: Connection) -> BoxFuture<'static, Result<()>>;
}

pub(crate) type PairSlot = Arc<Mutex<Option<Arc<dyn PairResponder>>>>;

#[derive(Clone)]
pub(crate) struct PairProtocol(pub PairSlot);

impl fmt::Debug for PairProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PairProtocol")
    }
}

impl ProtocolHandler for PairProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let responder = self.0.lock().unwrap().clone();
        match responder {
            Some(r) => r
                .respond(conn)
                .await
                .map_err(|e| AcceptError::from_err(std::io::Error::other(format!("{e:#}")))),
            None => {
                conn.close(2u32.into(), b"not pairing");
                Ok(())
            }
        }
    }
}

// ---------- inviter ----------

struct Invite {
    secret: [u8; 32],
    expires_ms: i64,
    role: Role,
    decide: DecideFn,
    failures: u32,
}

#[derive(Default)]
pub(crate) struct Invites {
    by_id: HashMap<[u8; 8], Invite>,
    /// Recent failed proofs, per joining device.
    failed: crate::limit::FailureLimit,
}

struct Inviter<L: LibraryStore> {
    inner: Weak<Inner<L>>,
    invites: Arc<Mutex<Invites>>,
}

impl<L: LibraryStore> PairResponder for Inviter<L> {
    fn respond(&self, conn: Connection) -> BoxFuture<'static, Result<()>> {
        let inner = self.inner.upgrade();
        let invites = self.invites.clone();
        Box::pin(async move {
            let inner = inner.ok_or_else(|| anyhow!("library closed"))?;
            let ln = LibraryNode { inner };
            let result = respond(&ln, &invites, &conn).await;
            if let Err(e) = &result {
                ln.node().emit(NodeEvent::PairingFailed {
                    reason: format!("{e:#}"),
                });
            }
            result
        })
    }
}

async fn respond<L: LibraryStore>(
    ln: &LibraryNode<L>,
    invites: &Arc<Mutex<Invites>>,
    conn: &Connection,
) -> Result<()> {
    let joiner = from_iroh_id(&conn.remote_id());
    let me = ln.device();
    let (mut send, mut recv) = conn.accept_bi().await?;
    let hello: ToInviter = tokio::time::timeout(HELLO_TIMEOUT, read_msg(&mut recv))
        .await
        .map_err(|_| anyhow!("no hello"))??;
    let ToInviter::Hello {
        ticket_id,
        proof: their_proof,
        nonce: nj,
        name,
    } = hello
    else {
        bail!("expected hello");
    };

    // Check and consume the ticket before anything else happens.
    let now = ln.now_ms();
    let accepted: Result<Invite, &'static str> = {
        let mut inv = invites.lock().unwrap();
        let peer = conn.remote_id();
        if !inv.failed.allows(&peer, now, MAX_FAILED_PER_MINUTE) {
            Err("too many failed attempts, try again later")
        } else {
            match inv.by_id.get_mut(&ticket_id) {
                None => {
                    inv.failed.record(peer, now);
                    Err(EXPIRED)
                }
                Some(i) if now as i64 >= i.expires_ms => {
                    inv.by_id.remove(&ticket_id);
                    Err(EXPIRED)
                }
                Some(i) => {
                    let expected = proof(&i.secret, &joiner, &me, &nj);
                    if expected == blake3::Hash::from(their_proof) {
                        Ok(inv.by_id.remove(&ticket_id).expect("present"))
                    } else {
                        i.failures += 1;
                        let burned = i.failures >= MAX_FAILED_PROOFS;
                        inv.failed.record(peer, now);
                        if burned {
                            inv.by_id.remove(&ticket_id);
                        }
                        Err(REJECTED)
                    }
                }
            }
        }
    };
    let invite = match accepted {
        Ok(i) => i,
        Err(reason) => {
            let _ = write_msg(&mut send, &ToJoiner::Refused(reason.into())).await;
            let _ = send.finish();
            let _ = tokio::time::timeout(Duration::from_secs(2), conn.closed()).await;
            bail!("{reason}");
        }
    };

    let ni: [u8; 16] = random();
    write_msg(
        &mut send,
        &ToJoiner::Challenge {
            nonce: ni,
            role: invite.role,
        },
    )
    .await?;
    let code = PairingCode {
        sas: sas(&invite.secret, &joiner, &me, &nj, &ni),
        peer_name: name.clone(),
        peer_role: invite.role,
    };
    ln.node().emit(NodeEvent::PairingCode {
        sas: code.sas.clone(),
        peer_name: code.peer_name.clone(),
        peer_role: code.peer_role,
    });
    let mine = (invite.decide)(code);
    let theirs = async {
        match read_msg::<ToInviter>(&mut recv).await? {
            ToInviter::Confirm(ok) => Ok::<bool, anyhow::Error>(ok),
            _ => bail!("expected confirmation"),
        }
    };
    // Either side declining ends it at once; otherwise wait for both.
    let decide_both = async {
        tokio::pin!(mine, theirs);
        let (mut mine_ok, mut theirs_ok): (Option<bool>, Option<bool>) = (None, None);
        while mine_ok.is_none() || theirs_ok.is_none() {
            tokio::select! {
                m = &mut mine, if mine_ok.is_none() => mine_ok = Some(m),
                t = &mut theirs, if theirs_ok.is_none() => theirs_ok = Some(t?),
            }
            if mine_ok == Some(false) || theirs_ok == Some(false) {
                return Ok::<(bool, bool), anyhow::Error>((false, false));
            }
        }
        Ok((true, true))
    };
    let (mine, theirs) = tokio::time::timeout(DECISION_TIMEOUT, decide_both)
        .await
        .map_err(|_| anyhow!("timed out waiting for confirmation"))??;
    if !(mine && theirs) {
        let _ = write_msg(&mut send, &ToJoiner::Refused("pairing declined".into())).await;
        let _ = send.finish();
        let _ = tokio::time::timeout(Duration::from_secs(2), conn.closed()).await;
        bail!("pairing declined");
    }

    // Both confirmed: certify the device and hand over the library.
    let cap = ln.capability()?;
    let list = ln.store().device_list().await?;
    let inviter_admin = list
        .admins
        .iter()
        .find(|a| a.admin == me)
        .cloned()
        .ok_or_else(|| anyhow!("this device is no longer an admin"))?;
    let cert = issue_device_cert(
        &ln.node().device_key(),
        cap.identity_id(),
        joiner,
        invite.role,
        &name,
        ln.holdings_hlc(),
        false,
    );
    ln.store()
        .apply_device_change(SignedDeviceChange::Device(cert.clone()))
        .await?;
    let grant = Grant {
        cert,
        inviter_admin,
        cap,
        peers: vec![from_endpoint_addr(&ln.node().addr())],
    };
    write_msg(&mut send, &ToJoiner::Granted(Box::new(grant))).await?;
    send.finish()?;
    let _ = tokio::time::timeout(Duration::from_secs(5), conn.closed()).await;
    ln.node().emit(NodeEvent::PairingDone { peer: joiner });
    Ok(())
}

impl<L: LibraryStore> LibraryNode<L> {
    /// Offer to pair one new device as `role`: returns the ticket to show (QR/text), valid for
    /// `ttl` and one use. `decide` is asked to confirm the six-digit code. Needs an admin device.
    pub async fn start_pairing(
        &self,
        role: Role,
        ttl: Duration,
        decide: DecideFn,
    ) -> Result<PairTicket> {
        let list = self.store().device_list().await?;
        ensure!(
            list.admins.iter().any(|a| a.admin == self.device()),
            "only an admin device can pair new devices"
        );
        let secret: [u8; 32] = random();
        let expires = self.now_ms() as i64 + ttl.as_millis() as i64;
        let ticket = PairTicket {
            addr: from_endpoint_addr(&self.node().addr()),
            secret,
            expires: Timestamp(expires),
            role,
        };
        self.inner.invites.lock().unwrap().by_id.insert(
            ticket.id(),
            Invite {
                secret,
                expires_ms: expires,
                role,
                decide,
                failures: 0,
            },
        );
        let responder: Arc<dyn PairResponder> = Arc::new(Inviter {
            inner: Arc::downgrade(&self.inner),
            invites: self.inner.invites.clone(),
        });
        *self.node().inner_pair_slot().lock().unwrap() = Some(responder);
        Ok(ticket)
    }
}

// ---------- joiner ----------

impl Node {
    /// Pair this device into the library the ticket belongs to. `decide` is asked to confirm the
    /// six-digit code; on success the returned grant holds the library capability and this
    /// device's certificate (already recorded by the inviter).
    pub async fn pair_join(
        &self,
        ticket: &PairTicket,
        name: &str,
        decide: DecideFn,
    ) -> Result<PairGrant> {
        let result = self.pair_join_inner(ticket, name, decide).await;
        match &result {
            Ok(grant) => self.emit(NodeEvent::PairingDone {
                peer: grant.cert.added_by,
            }),
            Err(e) => self.emit(NodeEvent::PairingFailed {
                reason: format!("{e:#}"),
            }),
        }
        result
    }

    async fn pair_join_inner(
        &self,
        ticket: &PairTicket,
        name: &str,
        decide: DecideFn,
    ) -> Result<PairGrant> {
        let me = from_iroh_id(&self.id());
        let inviter = ticket.addr.id;
        let addr = to_endpoint_addr(&ticket.addr)?;
        let conn = self
            .endpoint()
            .connect(addr, PAIR_ALPN)
            .await
            .context("could not reach the device that made this ticket")?;
        let (mut send, mut recv) = conn.open_bi().await?;
        let nj: [u8; 16] = random();
        write_msg(
            &mut send,
            &ToInviter::Hello {
                ticket_id: ticket.id(),
                proof: *proof(&ticket.secret, &me, &inviter, &nj).as_bytes(),
                nonce: nj,
                name: name.to_string(),
            },
        )
        .await?;
        let (ni, role) = match read_msg::<ToJoiner>(&mut recv).await? {
            ToJoiner::Challenge { nonce, role } => (nonce, role),
            ToJoiner::Refused(why) => bail!("{why}"),
            ToJoiner::Granted(_) => bail!("unexpected grant"),
        };
        ensure!(
            role == ticket.role,
            "the offered role does not match the ticket"
        );
        let code = PairingCode {
            sas: sas(&ticket.secret, &me, &inviter, &nj, &ni),
            peer_name: name.to_string(),
            peer_role: role,
        };
        self.emit(NodeEvent::PairingCode {
            sas: code.sas.clone(),
            peer_name: code.peer_name.clone(),
            peer_role: code.peer_role,
        });
        let ok = decide(code).await;
        write_msg(&mut send, &ToInviter::Confirm(ok)).await?;
        send.finish()?;
        let grant = match read_msg::<ToJoiner>(&mut recv).await? {
            ToJoiner::Granted(g) => g,
            ToJoiner::Refused(why) => bail!("{why}"),
            ToJoiner::Challenge { .. } => bail!("unexpected challenge"),
        };
        conn.close(0u32.into(), b"done");

        // Trust nothing the inviter says beyond what chains to the library's root key.
        let mut chain = TrustChain::new(grant.cap.root, grant.cap.identity_id());
        chain
            .check_admin(&grant.inviter_admin)
            .map_err(|e| anyhow!("inviter's admin certificate is invalid: {e}"))?;
        ensure!(
            grant.inviter_admin.admin == inviter,
            "admin certificate is for another device"
        );
        chain.add_admin(grant.inviter_admin.clone());
        chain
            .check_device_cert(&grant.cert)
            .map_err(|e| anyhow!("device certificate is invalid: {e}"))?;
        ensure!(
            grant.cert.device == me && !grant.cert.revoked,
            "certificate is not for this device"
        );
        ensure!(grant.cert.role == role, "certificate has a different role");
        Ok(PairGrant {
            cap: grant.cap,
            cert: grant.cert,
            role,
            peers: grant.peers,
        })
    }
}
