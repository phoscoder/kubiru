//! Where cluster tokens are kept.
//!
//! Tokens go to the operating system's credential store (Keychain on macOS,
//! Credential Manager on Windows, the Secret Service on Linux), never to the
//! SQLite file. [`MemoryVault`] exists for tests and for running without a
//! keychain.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::cluster::ClusterId;
use crate::secret::Secret;

/// Service name the tokens are filed under in the keychain.
pub const KEYCHAIN_SERVICE: &str = "dev.kubiru.Kubiru";

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error(
        "the system keychain is not available ({0}). On Linux, make sure a Secret Service \
         provider such as GNOME Keyring or KWallet is running."
    )]
    Unavailable(String),
    #[error("the system keychain refused the request: {0}")]
    Denied(String),
}

/// Storage for cluster bearer tokens.
pub trait TokenVault: Send + Sync {
    fn get(&self, cluster: ClusterId) -> Result<Option<Secret>, VaultError>;
    fn set(&self, cluster: ClusterId, token: &Secret) -> Result<(), VaultError>;
    /// Remove the token. Removing a token that is not there is not an error.
    fn delete(&self, cluster: ClusterId) -> Result<(), VaultError>;
}

/// The operating system's credential store.
#[derive(Debug, Default)]
pub struct KeyringVault;

impl KeyringVault {
    fn entry(cluster: ClusterId) -> Result<keyring::Entry, VaultError> {
        keyring::Entry::new(KEYCHAIN_SERVICE, &format!("cluster-{cluster}")).map_err(map_error)
    }
}

impl TokenVault for KeyringVault {
    fn get(&self, cluster: ClusterId) -> Result<Option<Secret>, VaultError> {
        match Self::entry(cluster)?.get_password() {
            Ok(token) => Ok(Some(Secret::new(token))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(map_error(e)),
        }
    }

    fn set(&self, cluster: ClusterId, token: &Secret) -> Result<(), VaultError> {
        Self::entry(cluster)?
            .set_password(token.expose())
            .map_err(map_error)
    }

    fn delete(&self, cluster: ClusterId) -> Result<(), VaultError> {
        match Self::entry(cluster)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(map_error(e)),
        }
    }
}

fn map_error(error: keyring::Error) -> VaultError {
    match error {
        keyring::Error::PlatformFailure(e) | keyring::Error::NoStorageAccess(e) => {
            VaultError::Unavailable(e.to_string())
        }
        other => VaultError::Denied(other.to_string()),
    }
}

/// Tokens held in process memory only; they are gone when the app exits.
#[derive(Debug, Default)]
pub struct MemoryVault {
    tokens: Mutex<HashMap<ClusterId, Secret>>,
}

impl MemoryVault {
    fn tokens(&self) -> std::sync::MutexGuard<'_, HashMap<ClusterId, Secret>> {
        // A panic while holding the lock cannot leave the map half-written.
        self.tokens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl TokenVault for MemoryVault {
    fn get(&self, cluster: ClusterId) -> Result<Option<Secret>, VaultError> {
        Ok(self.tokens().get(&cluster).cloned())
    }

    fn set(&self, cluster: ClusterId, token: &Secret) -> Result<(), VaultError> {
        self.tokens().insert(cluster, token.clone());
        Ok(())
    }

    fn delete(&self, cluster: ClusterId) -> Result<(), VaultError> {
        self.tokens().remove(&cluster);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_vault_round_trips() {
        let vault = MemoryVault::default();
        let id = ClusterId(7);
        assert!(vault.get(id).unwrap().is_none());
        vault.set(id, &Secret::new("t0k3n")).unwrap();
        assert_eq!(vault.get(id).unwrap().unwrap().expose(), "t0k3n");
        vault.delete(id).unwrap();
        vault.delete(id).unwrap();
        assert!(vault.get(id).unwrap().is_none());
    }
}
