//! The saved clusters, with their tokens.
//!
//! [`Registry`] keeps the SQLite row and the keychain entry for a cluster in
//! step: a cluster is only created once its token is safely stored, and
//! deleting it removes both.

use std::sync::{Arc, Mutex, MutexGuard};

use crate::cluster::{Cluster, ClusterDraft, ClusterId, TokenRequirement, ValidationErrors};
use crate::secret::Secret;
use crate::store::{Store, StoreError};
use crate::vault::{TokenVault, VaultError};

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("{0}")]
    Invalid(#[from] ValidationErrors),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Vault(#[from] VaultError),
    #[error("no token is stored for this cluster; edit it and paste a token")]
    MissingToken,
}

/// A cluster together with the token used to talk to it.
#[derive(Clone, Debug)]
pub struct Credentials {
    pub cluster: Cluster,
    pub token: Secret,
}

/// Thread-safe access to saved clusters. Keychain calls can block on an
/// unlock prompt, so call these methods off the UI thread.
pub struct Registry {
    store: Mutex<Store>,
    vault: Arc<dyn TokenVault>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry").finish_non_exhaustive()
    }
}

impl Registry {
    pub fn new(store: Store, vault: Arc<dyn TokenVault>) -> Self {
        Self {
            store: Mutex::new(store),
            vault,
        }
    }

    /// Run `f` with exclusive access to the store, for preferences.
    pub fn with_store<R>(&self, f: impl FnOnce(&mut Store) -> R) -> R {
        f(&mut self.store())
    }

    fn store(&self) -> MutexGuard<'_, Store> {
        self.store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn clusters(&self) -> Result<Vec<Cluster>, RegistryError> {
        Ok(self.store().clusters()?)
    }

    pub fn cluster(&self, id: ClusterId) -> Result<Cluster, RegistryError> {
        Ok(self.store().cluster(id)?)
    }

    /// Validate and save a new cluster and its token.
    pub fn create(&self, draft: &ClusterDraft) -> Result<Cluster, RegistryError> {
        let valid = draft.validate(TokenRequirement::Required)?;
        let token = valid.token.clone().ok_or(RegistryError::MissingToken)?;
        let store = self.store();
        let cluster = store.insert_cluster(&valid)?;
        if let Err(e) = self.vault.set(cluster.id, &token) {
            // Without its token the row is useless; take it back out.
            drop(store);
            let _ = self.store().delete_cluster(cluster.id);
            return Err(e.into());
        }
        Ok(cluster)
    }

    /// Validate and save changes. A blank token keeps the stored one.
    pub fn update(&self, id: ClusterId, draft: &ClusterDraft) -> Result<Cluster, RegistryError> {
        let valid = draft.validate(TokenRequirement::KeepIfBlank)?;
        let cluster = self.store().update_cluster(id, &valid)?;
        if let Some(token) = &valid.token {
            self.vault.set(id, token)?;
        }
        Ok(cluster)
    }

    /// Delete a cluster and its token.
    pub fn delete(&self, id: ClusterId) -> Result<(), RegistryError> {
        self.vault.delete(id)?;
        self.store().delete_cluster(id)?;
        Ok(())
    }

    /// Everything needed to open a connection to `id`.
    pub fn credentials(&self, id: ClusterId) -> Result<Credentials, RegistryError> {
        let cluster = self.cluster(id)?;
        let token = self.vault.get(id)?.ok_or(RegistryError::MissingToken)?;
        Ok(Credentials { cluster, token })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::MemoryVault;

    fn registry() -> (Registry, Arc<MemoryVault>) {
        let vault = Arc::new(MemoryVault::default());
        (
            Registry::new(Store::open_in_memory().unwrap(), vault.clone()),
            vault,
        )
    }

    fn draft(name: &str, token: &str) -> ClusterDraft {
        ClusterDraft {
            name: name.into(),
            api_url: "https://k8s.example.com".into(),
            token: Secret::new(token),
            ..Default::default()
        }
    }

    #[test]
    fn creating_stores_the_token_in_the_vault_not_the_database() {
        let (registry, vault) = registry();
        let cluster = registry.create(&draft("prod", "secret-1")).unwrap();
        assert_eq!(vault.get(cluster.id).unwrap().unwrap().expose(), "secret-1");
        assert_eq!(
            registry.credentials(cluster.id).unwrap().token.expose(),
            "secret-1"
        );
    }

    #[test]
    fn invalid_drafts_are_not_saved() {
        let (registry, _) = registry();
        assert!(matches!(
            registry.create(&draft("", "x")),
            Err(RegistryError::Invalid(_))
        ));
        assert!(registry.clusters().unwrap().is_empty());
    }

    #[test]
    fn a_blank_token_on_update_keeps_the_old_one() {
        let (registry, _) = registry();
        let cluster = registry.create(&draft("prod", "first")).unwrap();
        registry.update(cluster.id, &draft("prod", "")).unwrap();
        assert_eq!(
            registry.credentials(cluster.id).unwrap().token.expose(),
            "first"
        );
        registry
            .update(cluster.id, &draft("prod", "second"))
            .unwrap();
        assert_eq!(
            registry.credentials(cluster.id).unwrap().token.expose(),
            "second"
        );
    }

    #[test]
    fn deleting_removes_row_and_token() {
        let (registry, vault) = registry();
        let cluster = registry.create(&draft("prod", "t")).unwrap();
        registry.delete(cluster.id).unwrap();
        assert!(vault.get(cluster.id).unwrap().is_none());
        assert!(registry.clusters().unwrap().is_empty());
    }

    struct BrokenVault;
    impl TokenVault for BrokenVault {
        fn get(&self, _: ClusterId) -> Result<Option<Secret>, VaultError> {
            Err(VaultError::Unavailable("test".into()))
        }
        fn set(&self, _: ClusterId, _: &Secret) -> Result<(), VaultError> {
            Err(VaultError::Unavailable("test".into()))
        }
        fn delete(&self, _: ClusterId) -> Result<(), VaultError> {
            Err(VaultError::Unavailable("test".into()))
        }
    }

    #[test]
    fn a_failed_token_write_rolls_back_the_row() {
        let registry = Registry::new(Store::open_in_memory().unwrap(), Arc::new(BrokenVault));
        assert!(matches!(
            registry.create(&draft("prod", "t")),
            Err(RegistryError::Vault(_))
        ));
        assert!(registry.clusters().unwrap().is_empty());
    }
}
