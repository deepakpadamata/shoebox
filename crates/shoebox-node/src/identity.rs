//! Identity operations on a library node (M2-01/02): certifying this device and the recovery kit.

use crate::LibraryNode;
use anyhow::Result;
use shoebox_core::identity::{issue_admin_cert, issue_device_cert, KeyPair};
use shoebox_core::model::Role;
use shoebox_core::recovery::RecoveryKit;
use shoebox_core::store::{LibraryStore, SignedDeviceChange};

pub const RECOVERY_KIT_SAVED_KEY: &str = "recovery_kit_saved";

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
        Ok(RecoveryKit::new(cap.library, self.now_ms(), root.clone()))
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
        let admin = issue_admin_cert(root, cap.library, me, self.holdings_hlc(), None);
        store
            .apply_device_change(SignedDeviceChange::Admin(admin))
            .await?;
        let cert = issue_device_cert(
            root,
            cap.library,
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
