//! Revocation with rotation (M2-03b): after an admin revokes a device it moves the library to a
//! fresh epoch (`LibraryStore::begin_rotation`) and tells every other member, who follow with
//! `adopt_rotation`. The revoked device keeps the old id and can reach nobody who moved.
//!
//! The move is announced with a signed [`RotationNotice`] (see `core::identity`): it carries the
//! revocation and the admin's certificate, so a member that was offline can verify both. Members
//! keep the latest notice (on disk) and hand it to any device that connects, so a device that was
//! away catches up from whoever it meets first.

use crate::addr::{from_iroh_id, to_endpoint_addr, to_iroh_id};
use crate::library_node::Inner;
use crate::node::{NodeEvent, ROTATE_ALPN};
use crate::LibraryNode;
use anyhow::{anyhow, bail, ensure, Context, Result};
use futures::future::BoxFuture;
use iroh::endpoint::Connection;
use shoebox_core::identity::{
    issue_admin_cert, issue_device_cert, issue_root_handover, issue_rotation_notice, EpochNotice,
    KeyPair, RootHandover, RotationNotice,
};
use shoebox_core::ids::EndpointId;
use shoebox_core::recovery::RecoveryKit;
use shoebox_core::store::{LibraryStore, SignedDeviceChange};
use std::path::Path;
use std::sync::{Arc, Weak};
use std::time::Duration;

const FILE: &str = "epoch-notice.bin";
const MAX_NOTICE: usize = 1 << 20;
const PUSH_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) fn load_notice(data_dir: &Path) -> Option<EpochNotice> {
    postcard::from_bytes(&std::fs::read(data_dir.join(FILE)).ok()?).ok()
}

fn save_notice(data_dir: &Path, notice: &EpochNotice) {
    if let Ok(bytes) = postcard::to_stdvec(notice) {
        let tmp = data_dir.join("rotation-notice.tmp");
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, data_dir.join(FILE));
        }
    }
}

struct Responder<L: LibraryStore>(Weak<Inner<L>>);

impl<L: LibraryStore> crate::ext::ConnHandler for Responder<L> {
    fn handle(&self, conn: Connection) -> BoxFuture<'static, Result<()>> {
        let inner = self.0.upgrade();
        Box::pin(async move {
            let ln = LibraryNode {
                inner: inner.ok_or_else(|| anyhow!("library closed"))?,
            };
            let (mut send, mut recv) = conn.accept_bi().await?;
            let bytes = recv.read_to_end(MAX_NOTICE).await?;
            let reply: &[u8] = match postcard::from_bytes::<EpochNotice>(&bytes) {
                Ok(notice) => match ln.accept_epoch_notice(&notice).await {
                    Ok(true) => b"1",
                    Ok(false) => b"0",
                    Err(e) => {
                        tracing::debug!("rejected a rotation notice: {e:#}");
                        b"x"
                    }
                },
                Err(_) => b"x",
            };
            send.write_all(reply).await?;
            send.finish()?;
            let _ = tokio::time::timeout(Duration::from_secs(2), conn.closed()).await;
            Ok(())
        })
    }
}

impl<L: LibraryStore> LibraryNode<L> {
    pub(crate) fn start_rotation_support(&self) {
        let responder: Arc<dyn crate::ext::ConnHandler> =
            Arc::new(Responder(Arc::downgrade(&self.inner)));
        *self.node().rotate_slot().lock().unwrap() = Some(responder);
        self.start_recovery_responder();
        // Hand the latest notice to every device that connects.
        let weak = Arc::downgrade(&self.inner);
        let mut events = self.node().events();
        tokio::spawn(async move {
            loop {
                let id = match events.recv().await {
                    Ok(NodeEvent::PeerConnected { id }) => from_iroh_id(&id),
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                };
                let Some(inner) = weak.upgrade() else { break };
                let ln = LibraryNode { inner };
                if ln.inner.notice.lock().unwrap().is_some() {
                    tokio::spawn(async move {
                        let _ = ln.push_notice(&id).await;
                    });
                }
            }
        });
    }

    /// The notice for the latest epoch change this device took part in, if any.
    pub fn epoch_notice(&self) -> Option<EpochNotice> {
        self.inner.notice.lock().unwrap().clone()
    }

    /// [`LibraryNode::epoch_notice`] when the latest change was a revocation.
    pub fn rotation_notice(&self) -> Option<RotationNotice> {
        match self.epoch_notice()? {
            EpochNotice::Revocation(n) => Some(n),
            EpochNotice::Root(_) => None,
        }
    }

    fn set_capability(&self, cap: shoebox_core::store::LibraryCapability) {
        *self.inner.cap.lock().unwrap() = Some(cap.clone());
        self.node().emit(NodeEvent::LibraryRotated { cap });
    }

    fn remember_notice(&self, notice: &EpochNotice) {
        *self.inner.notice.lock().unwrap() = Some(notice.clone());
        self.inner.notice_acked.lock().unwrap().clear();
        save_notice(&self.node().config().data_dir, notice);
    }

    /// Revoke `device` and move the library to a fresh epoch so the revoked device is locked out
    /// of the replicated data even if it reaches a member that has not heard of the revocation.
    /// This device must be an admin. Other members are told right away when reachable, and when
    /// they next connect otherwise.
    pub async fn revoke_and_rotate(&self, device: &EndpointId) -> Result<RotationNotice> {
        self.revoke_device(device).await?;
        self.rotate_after_revocation(device).await
    }

    /// [`LibraryNode::revoke_and_rotate`] with the library's root key (from the recovery phrase):
    /// needed to remove an admin device (A-085).
    pub async fn revoke_and_rotate_with_root(
        &self,
        root: &KeyPair,
        device: &EndpointId,
    ) -> Result<RotationNotice> {
        self.revoke_device_with_root(root, device).await?;
        self.rotate_after_revocation(device).await
    }

    /// [`LibraryNode::revoke_and_rotate_with_root`] with the root key taken from the recovery
    /// phrase (checked against this library's root before anything is written).
    pub async fn revoke_and_rotate_with_phrase(
        &self,
        phrase: &str,
        device: &EndpointId,
    ) -> Result<RotationNotice> {
        let root =
            shoebox_core::recovery::RecoveryKit::key_from_phrase(phrase, &self.capability()?.root)?;
        self.revoke_and_rotate_with_root(&root, device).await
    }

    async fn rotate_after_revocation(&self, device: &EndpointId) -> Result<RotationNotice> {
        let old = self.capability()?;
        let new = self.store().begin_rotation().await?;
        let list = self.store().device_list().await?;
        let admin = list
            .admins
            .iter()
            .find(|a| a.admin == self.device())
            .cloned()
            .ok_or_else(|| anyhow!("only an admin device can rotate the library"))?;
        let revocation = list
            .devices
            .iter()
            .find(|d| d.device == *device && d.revoked)
            .cloned()
            .ok_or_else(|| anyhow!("the revocation is not in the device list"))?;
        let notice = issue_rotation_notice(
            &self.node().device_key(),
            old.identity_id(),
            old.library,
            new.clone(),
            revocation,
            admin,
        );
        self.remember_notice(&EpochNotice::Revocation(notice.clone()));
        self.set_capability(new);
        self.announce_notice();
        Ok(notice)
    }

    /// Replace the library's root key, for when the recovery phrase may have leaked. `phrase` is
    /// the current one. A new root key is made; every device and admin that is a member now is
    /// re-certified by it in a new epoch, which the others follow (the old phrase opens nothing
    /// there). Devices that were removed are left behind. Returns the new recovery kit: show it
    /// once, it is the only copy.
    pub async fn rotate_root(&self, phrase: &str) -> Result<RecoveryKit> {
        let old_cap = self.capability()?;
        let old_root = RecoveryKit::key_from_phrase(phrase, &old_cap.root)?;
        let new_root = KeyPair::generate();
        let identity = old_cap.identity_id();
        let me = self.device();
        let list = self.store().device_list().await?;
        anyhow::ensure!(
            list.devices.iter().any(|d| d.device == me && !d.revoked),
            "this device is not a member of the library"
        );
        // Who is an admin and who is a member now, signed by the new root.
        let mut certs = vec![];
        let mut admins: Vec<EndpointId> = list.admins.iter().map(|a| a.admin).collect();
        if !admins.contains(&me) {
            admins.push(me); // whoever holds the phrase can act on what follows
        }
        for admin in admins {
            let expires = list
                .admins
                .iter()
                .find(|a| a.admin == admin)
                .and_then(|a| a.expires_at);
            certs.push(SignedDeviceChange::Admin(issue_admin_cert(
                &new_root,
                identity,
                admin,
                self.holdings_hlc(),
                expires,
            )));
        }
        for d in list.devices.iter().filter(|d| !d.revoked) {
            certs.push(SignedDeviceChange::Device(issue_device_cert(
                &new_root,
                identity,
                d.device,
                d.role,
                &d.name,
                self.holdings_hlc(),
                false,
            )));
        }
        let new_cap = self
            .store()
            .begin_root_rotation(new_root.root_public(), certs)
            .await?;
        let handover = issue_root_handover(
            &old_root,
            &new_root,
            identity,
            old_cap.library,
            new_cap.clone(),
        );
        // The new phrase has not been saved yet.
        self.set_recovery_kit_saved(false).await?;
        self.remember_notice(&EpochNotice::Root(handover));
        self.set_capability(new_cap);
        self.announce_notice();
        Ok(RecoveryKit::new(identity, self.now_ms(), new_root))
    }

    /// Push the current notice to every non-revoked member (in the background).
    fn announce_notice(&self) {
        let ln = self.clone();
        tokio::spawn(async move {
            let Ok(list) = ln.store().device_list().await else {
                return;
            };
            for d in list
                .devices
                .iter()
                .filter(|d| !d.revoked && d.device != ln.device())
            {
                let (ln, id) = (ln.clone(), d.device);
                tokio::spawn(async move {
                    let _ = ln.push_notice(&id).await;
                });
            }
        });
    }

    /// Offer the current notice to `peer`; returns whether it moved because of it.
    pub(crate) async fn push_notice(&self, peer: &EndpointId) -> Result<bool> {
        let Some(notice) = self.epoch_notice() else {
            return Ok(false);
        };
        if self.inner.notice_acked.lock().unwrap().contains(peer) {
            return Ok(false);
        }
        // Never talk rotation to the revoked device, or to anybody the device list does not know.
        let list = self.store().device_list().await?;
        ensure!(
            list.devices.iter().any(|d| d.device == *peer && !d.revoked),
            "not a current member"
        );
        let addr = match self.inner.peers.lock().unwrap().get(peer) {
            Some(p) => to_endpoint_addr(&p.addr).map_err(|e| anyhow!("{e}"))?,
            None => iroh::EndpointAddr::from(to_iroh_id(peer)?),
        };
        let push = async {
            let conn = self
                .node()
                .endpoint()
                .connect(addr, ROTATE_ALPN)
                .await
                .context("connecting")?;
            let (mut send, mut recv) = conn.open_bi().await?;
            send.write_all(&postcard::to_stdvec(&notice)?).await?;
            send.finish()?;
            let reply = recv.read_to_end(8).await?;
            conn.close(0u32.into(), b"done");
            Ok::<bool, anyhow::Error>(reply == b"1")
        };
        let moved = tokio::time::timeout(PUSH_TIMEOUT, push)
            .await
            .map_err(|_| anyhow!("timed out"))??;
        self.inner.notice_acked.lock().unwrap().insert(*peer);
        Ok(moved)
    }

    /// Test hook: feed a notice to this node as if a peer had pushed it.
    #[doc(hidden)]
    pub async fn accept_notice_for_test(&self, notice: &RotationNotice) -> Result<bool> {
        self.accept_notice(notice).await
    }

    /// Test hook: [`LibraryNode::accept_notice_for_test`] for either kind of notice.
    #[doc(hidden)]
    pub async fn accept_epoch_notice_for_test(&self, notice: &EpochNotice) -> Result<bool> {
        self.accept_epoch_notice(notice).await
    }

    pub(crate) async fn accept_epoch_notice(&self, notice: &EpochNotice) -> Result<bool> {
        match notice {
            EpochNotice::Revocation(n) => self.accept_notice(n).await,
            EpochNotice::Root(h) => self.accept_handover(h).await,
        }
    }

    /// Follow a root handover: from the epoch this device is in, signed by the root it trusts
    /// now and by the new one. The first valid handover wins: once this device has moved, a
    /// competing one (from the same old root) is for an epoch it has left.
    async fn accept_handover(&self, handover: &RootHandover) -> Result<bool> {
        let cap = self.capability()?;
        if handover.new_cap.library == cap.library {
            return Ok(false); // already there
        }
        ensure!(
            handover.old == cap.library,
            "the handover is for another epoch of this library"
        );
        handover
            .verify(&cap.root, cap.identity_id())
            .map_err(|e| anyhow!("invalid root handover: {e}"))?;
        self.store().adopt_root_rotation(&handover.new_cap).await?;
        self.remember_notice(&EpochNotice::Root(handover.clone()));
        self.set_capability(handover.new_cap.clone());
        self.node().emit(NodeEvent::RecoveryPhraseReplaced);
        tracing::warn!("the library's recovery phrase was replaced on another device");
        for id in self.peers() {
            let _ = self.inner.nudge.send(id);
        }
        Ok(true)
    }

    /// Verify and follow a rotation announced by an admin. `Ok(true)` if this device moved.
    pub(crate) async fn accept_notice(&self, notice: &RotationNotice) -> Result<bool> {
        let cap = self.capability()?;
        if notice.new_cap.library == cap.library {
            return Ok(false); // already there
        }
        // Normally a notice moves us on from the epoch we are in. One exception: we followed an
        // admin's notice, and this one, from the epoch we left, removes that admin with the root
        // key. The admin was stolen; its move must not keep us from the real one (A-085).
        let followed_a_removed_admin = self.rotation_notice().is_some_and(|ours| {
            ours.new_cap.library == cap.library
                && ours.old == notice.old
                && notice.revocation.device == ours.admin.admin
                && notice.revocation.added_by.0 == cap.root.0
        });
        ensure!(
            notice.old == cap.library || followed_a_removed_admin,
            "the notice is for another epoch of this library"
        );
        notice
            .verify(&cap.root, cap.identity_id())
            .map_err(|e| anyhow!("invalid rotation notice: {e}"))?;
        // `verify` only sees the notice itself. What this device already knows counts too: a
        // notice from an admin it knows was revoked, or whose admin rights have expired by its own
        // clock, is refused (another member may forward one it adopted before it heard).
        let signer = notice.admin.admin;
        let now = self.now_ms() as i64;
        let list = self.store().device_list().await?;
        ensure!(
            !list.devices.iter().any(|d| d.device == signer && d.revoked),
            "invalid rotation notice: its signer was removed from this library"
        );
        ensure!(
            notice.admin.expires_at.is_none_or(|t| t.0 > now),
            "invalid rotation notice: its signer's admin rights have expired"
        );
        if notice.revocation.device == self.device() {
            bail!("this device was revoked");
        }
        // Learn the revocation (and the admin) the notice proves, then follow the move.
        let store = self.store();
        store
            .apply_device_change(SignedDeviceChange::Admin(notice.admin.clone()))
            .await?;
        store
            .apply_device_change(SignedDeviceChange::Device(notice.revocation.clone()))
            .await?;
        store.adopt_rotation(&notice.new_cap).await?;
        self.remember_notice(&EpochNotice::Revocation(notice.clone()));
        self.set_capability(notice.new_cap.clone());
        // Everything else comes back from the others.
        for id in self.peers() {
            let _ = self.inner.nudge.send(id);
        }
        Ok(true)
    }
}
