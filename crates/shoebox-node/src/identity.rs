//! Identity operations on a library node (M2-01/02): certifying this device and the recovery kit.

use crate::{LibraryNode, NodeEvent};
use anyhow::Result;
use shoebox_core::identity::{issue_admin_cert, issue_device_cert, KeyPair};
use shoebox_core::model::Role;
use shoebox_core::recovery::RecoveryKit;
use shoebox_core::store::{Change, LibraryStore, SignedDeviceChange};
use std::collections::HashSet;
use std::sync::Arc;

pub const RECOVERY_KIT_SAVED_KEY: &str = "recovery_kit_saved";

/// What this device's replica of the device list says about this device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Membership {
    /// No device list yet, or this device is not in it (yet).
    Unknown,
    Member,
    /// This device was revoked: it refuses everyone and stops syncing.
    Removed,
}

impl<L: LibraryStore> LibraryNode<L> {
    /// Make this device the library's first member and an admin, signed by `root`, and return
    /// the recovery kit. The root key is not kept: the caller must show the kit to the user and
    /// drop the key.
    pub async fn create_identity(
        &self,
        root: &KeyPair,
        role: Role,
        name: &str,
    ) -> Result<RecoveryKit> {
        let cap = self.capability()?;
        self.certify_self(root, role, name).await?;
        Ok(RecoveryKit::new(
            cap.identity_id(),
            self.now_ms(),
            root.clone(),
        ))
    }

    /// Rejoin an existing library with the recovery phrase: verify it against the library's
    /// root key *first* (nothing is written on failure), then certify this new device as an
    /// admin member.
    pub async fn restore_from_kit(&self, phrase: &str, role: Role, name: &str) -> Result<()> {
        let cap = self.capability()?;
        let root = RecoveryKit::key_from_phrase(phrase, &cap.root)?;
        self.certify_self(&root, role, name).await
    }

    async fn certify_self(&self, root: &KeyPair, role: Role, name: &str) -> Result<()> {
        let cap = self.capability()?;
        let me = self.device();
        let store = self.store();
        let admin = issue_admin_cert(root, cap.identity_id(), me, self.holdings_hlc(), None);
        store
            .apply_device_change(SignedDeviceChange::Admin(admin))
            .await?;
        let cert = issue_device_cert(
            root,
            cap.identity_id(),
            me,
            role,
            name,
            self.holdings_hlc(),
            false,
        );
        store
            .apply_device_change(SignedDeviceChange::Device(cert))
            .await?;
        Ok(())
    }

    /// Start enforcing the device list: from now on only certified, non-revoked devices may
    /// connect (M2-03), and a revocation of this device flips [`LibraryNode::membership`] to
    /// `Removed` and emits [`NodeEvent::RemovedFromLibrary`]. Re-evaluated on every device-list
    /// change. Until a device list exists nothing is refused, so a fresh joiner can still sync.
    pub async fn enforce_membership(&self) -> Result<()> {
        self.apply_membership().await?;
        let weak = Arc::downgrade(&self.inner);
        let mut changes = self.store().subscribe();
        tokio::spawn(async move {
            while let Some(change) = futures::StreamExt::next(&mut changes).await {
                if change != Change::DeviceListChanged {
                    continue;
                }
                let Some(inner) = weak.upgrade() else { break };
                let _ = (LibraryNode { inner }).apply_membership().await;
            }
        });
        Ok(())
    }

    pub async fn membership(&self) -> Result<Membership> {
        self.apply_membership().await?;
        Ok(*self.inner.membership.lock().unwrap())
    }

    async fn apply_membership(&self) -> Result<()> {
        let list = self.store().device_list().await?;
        let me = self.device();
        let mine = list.devices.iter().find(|d| d.device == me);
        let state = match mine {
            Some(c) if c.revoked => Membership::Removed,
            Some(_) => Membership::Member,
            None => Membership::Unknown,
        };
        let allowed: Option<HashSet<_>> = if list.devices.is_empty() {
            None
        } else if state == Membership::Removed {
            Some(HashSet::new())
        } else {
            Some(
                list.devices
                    .iter()
                    .filter(|d| !d.revoked)
                    .filter_map(|d| crate::to_iroh_id(&d.device).ok())
                    .collect(),
            )
        };
        self.node().set_allowed_peers(allowed);
        let previous = std::mem::replace(&mut *self.inner.membership.lock().unwrap(), state);
        if state == Membership::Removed && previous != Membership::Removed {
            self.node().emit(NodeEvent::RemovedFromLibrary);
        }
        Ok(())
    }

    /// Revoke `device` (signed by this device, which must be an admin). The revocation
    /// replicates; peers then refuse the device (see [`LibraryNode::enforce_membership`]).
    pub async fn revoke_device(&self, device: &shoebox_core::ids::EndpointId) -> Result<()> {
        let cap = self.capability()?;
        let list = self.store().device_list().await?;
        anyhow::ensure!(
            list.admins.iter().any(|a| a.admin == self.device()),
            "only an admin device can revoke devices"
        );
        let current = list
            .devices
            .iter()
            .find(|d| d.device == *device)
            .ok_or_else(|| anyhow::anyhow!("no such device in this library"))?;
        anyhow::ensure!(!current.revoked, "that device is already revoked");
        anyhow::ensure!(
            !list.admins.iter().any(|a| a.admin == *device),
            "that device is an admin: removing it needs the recovery phrase"
        );
        let cert = shoebox_core::identity::issue_device_cert(
            &self.node().device_key(),
            cap.identity_id(),
            *device,
            current.role,
            &current.name,
            self.holdings_hlc(),
            true,
        );
        self.store()
            .apply_device_change(SignedDeviceChange::Device(cert))
            .await?;
        Ok(())
    }

    /// Revoke `device` with the library's root key (from the recovery phrase): the only way to
    /// remove an admin device, and it works for any device (A-085). In the same step the root
    /// re-signs every cert `device` signed that is still some device's current cert, because once
    /// the root has removed an admin none of its own signatures count. If this device is not an
    /// admin yet, the root makes it one, so it can sign the rotation that follows.
    pub async fn revoke_device_with_root(
        &self,
        root: &KeyPair,
        device: &shoebox_core::ids::EndpointId,
    ) -> Result<()> {
        let cap = self.capability()?;
        anyhow::ensure!(
            root.root_public() == cap.root,
            "that recovery phrase belongs to another library"
        );
        anyhow::ensure!(*device != self.device(), "this device cannot remove itself");
        let store = self.store();
        let list = store.device_list().await?;
        let current = list
            .devices
            .iter()
            .find(|d| d.device == *device)
            .ok_or_else(|| anyhow::anyhow!("no such device in this library"))?
            .clone();
        anyhow::ensure!(!current.revoked, "that device is already revoked");
        if !list.admins.iter().any(|a| a.admin == self.device()) {
            let admin = issue_admin_cert(
                root,
                cap.identity_id(),
                self.device(),
                self.holdings_hlc(),
                None,
            );
            store
                .apply_device_change(SignedDeviceChange::Admin(admin))
                .await?;
        }
        // Keep what it signed first, then remove it, so no device drops out in between.
        for kept in list
            .devices
            .iter()
            .filter(|d| d.added_by == *device && d.device != *device)
        {
            let cert = issue_device_cert(
                root,
                cap.identity_id(),
                kept.device,
                kept.role,
                &kept.name,
                self.holdings_hlc(),
                kept.revoked,
            );
            store
                .apply_device_change(SignedDeviceChange::Device(cert))
                .await?;
        }
        let revocation = issue_device_cert(
            root,
            cap.identity_id(),
            *device,
            current.role,
            &current.name,
            self.holdings_hlc(),
            true,
        );
        store
            .apply_device_change(SignedDeviceChange::Device(revocation))
            .await?;
        Ok(())
    }

    /// This device was renamed (the node's config already has the new name). Tell the others
    /// that are online right away; if this device is an admin it also re-signs its own
    /// certificate, so the new name reaches devices that are offline now and the device list.
    pub async fn announce_rename(&self, name: &str) -> Result<()> {
        if let (Ok(cap), Ok(list)) = (self.capability(), self.store().device_list().await) {
            if list.admins.iter().any(|a| a.admin == self.device()) {
                if let Some(me) = list.devices.iter().find(|d| d.device == self.device()) {
                    if me.name != name && !me.revoked {
                        let cert = shoebox_core::identity::issue_device_cert(
                            &self.node().device_key(),
                            cap.identity_id(),
                            self.device(),
                            me.role,
                            name,
                            self.holdings_hlc(),
                            false,
                        );
                        self.store()
                            .apply_device_change(SignedDeviceChange::Device(cert))
                            .await?;
                    }
                }
            }
        }
        self.send_hello().await;
        Ok(())
    }

    /// Whether the user has saved the recovery kit (drives the nag banner).
    pub async fn recovery_kit_saved(&self) -> Result<bool> {
        Ok(self
            .store()
            .setting(RECOVERY_KIT_SAVED_KEY)
            .await?
            .is_some_and(|v| v == [1]))
    }

    pub async fn set_recovery_kit_saved(&self, saved: bool) -> Result<()> {
        self.store()
            .put_setting(RECOVERY_KIT_SAVED_KEY, vec![u8::from(saved)])
            .await?;
        Ok(())
    }
}
