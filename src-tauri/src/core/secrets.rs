//! Password storage. The app keeps each secret as its own entry in the OS credential store:
//! Keychain on macOS, Credential Manager on Windows, Secret Service (GNOME Keyring, KWallet) on
//! Linux. Tests and the dev bridge use memory only.
//!
//! Releases up to 0.3.0 kept secrets in `credentials.enc` in the data dir, encrypted with a key
//! built into the app. `migrate_legacy_file` moves those entries into the credential store once
//! and then removes the file.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;

use crate::core::error::{AppError, ErrorCode};

/// The pre-keychain password file. Read only by `migrate_legacy_file`.
pub const LEGACY_CREDENTIALS_FILE: &str = "credentials.enc";

/// The key of `credentials.enc`. Kept only to read old files during migration.
const KEY: [u8; 32] = [
    0x75, 0xb9, 0x55, 0x1d, 0x41, 0xdf, 0x71, 0xd9, 0x85, 0xe7, 0xd2, 0x3a, 0xd2, 0x32, 0xbb, 0xfb,
    0x07, 0x66, 0xc4, 0x33, 0xe4, 0x80, 0xc3, 0xf6, 0xdf, 0x85, 0x9b, 0xe0, 0x3f, 0x0a, 0x28, 0xc1,
];
/// File layout, base64-encoded: `VERSION` byte, 12-byte nonce, AES-256-GCM ciphertext of a JSON map.
const VERSION: u8 = 1;
const NONCE_LEN: usize = 12;

pub fn password_account(conn_id: &str) -> String {
    format!("conn:{conn_id}:password")
}

pub trait SecretStore: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<String>, AppError>;
    fn set(&self, account: &str, secret: &str) -> Result<(), AppError>;
    /// Deleting a missing entry is not an error.
    fn delete(&self, account: &str) -> Result<(), AppError>;
}

fn secret_err(e: impl std::fmt::Display) -> AppError {
    AppError::new(
        ErrorCode::SecretStore,
        "Could not use the system credential store (Keychain, Credential Manager or Secret Service)",
    )
    .with_detail(e.to_string())
}

/// One credential store entry per account, all under the same `service` name.
pub struct KeychainStore {
    service: String,
}

impl KeychainStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, account: &str) -> Result<keyring::Entry, AppError> {
        keyring::Entry::new(&self.service, account).map_err(secret_err)
    }
}

impl SecretStore for KeychainStore {
    fn get(&self, account: &str) -> Result<Option<String>, AppError> {
        match self.entry(account)?.get_password() {
            Ok(s) => Ok(Some(s)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(secret_err(e)),
        }
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), AppError> {
        self.entry(account)?
            .set_password(secret)
            .map_err(secret_err)
    }

    fn delete(&self, account: &str) -> Result<(), AppError> {
        match self.entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(secret_err(e)),
        }
    }
}

/// Moves the entries of `credentials.enc` in `data_dir` into `store`, then removes the file.
///
/// An entry that `store` already has is not overwritten: after a migration that stopped part way,
/// the user can have changed that secret in the app, and the file copy is older. On any error the
/// file stays, so the next start tries again and no secret is lost.
pub fn migrate_legacy_file(data_dir: &Path, store: &dyn SecretStore) -> Result<(), AppError> {
    let path = data_dir.join(LEGACY_CREDENTIALS_FILE);
    let Some(map) = read_legacy_file(&path)? else {
        return Ok(());
    };
    for (account, secret) in &map {
        if store.get(account)?.is_none() {
            store.set(account, secret)?;
        }
    }
    fs::remove_file(&path).map_err(|e| {
        AppError::new(
            ErrorCode::Io,
            format!("Could not remove {}", path.display()),
        )
        .with_detail(e.to_string())
    })
}

/// `None` when there is no file. A file that does not decrypt is an error, never an empty map:
/// treating it as empty would remove the file and every password in it.
fn read_legacy_file(path: &Path) -> Result<Option<BTreeMap<String, String>>, AppError> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(
                AppError::new(ErrorCode::Io, format!("Could not read {}", path.display()))
                    .with_detail(e.to_string()),
            );
        }
    };
    // No decode or parse detail: it could quote the decrypted content.
    let damaged = || {
        AppError::new(
            ErrorCode::SecretStore,
            format!(
                "{} is damaged. Remove it and enter your connection passwords again.",
                path.display()
            ),
        )
    };
    let raw = B64.decode(text.trim()).map_err(|_| damaged())?;
    if raw.len() < 1 + NONCE_LEN || raw[0] != VERSION {
        return Err(damaged());
    }
    let (nonce, body) = raw[1..].split_at(NONCE_LEN);
    let plain = Aes256Gcm::new(&KEY.into())
        .decrypt(Nonce::from_slice(nonce), body)
        .map_err(|_| damaged())?;
    serde_json::from_slice(&plain)
        .map(Some)
        .map_err(|_| damaged())
}

#[derive(Default)]
pub struct MemoryStore {
    map: Mutex<std::collections::HashMap<String, String>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, account: &str) -> Result<Option<String>, AppError> {
        Ok(self.map.lock().unwrap().get(account).cloned())
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), AppError> {
        self.map
            .lock()
            .unwrap()
            .insert(account.to_string(), secret.to_string());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), AppError> {
        self.map.lock().unwrap().remove(account);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_name_format() {
        assert_eq!(password_account("abc"), "conn:abc:password");
    }

    #[test]
    fn memory_store_round_trip() {
        let s = MemoryStore::new();
        assert_eq!(s.get("a").unwrap(), None);
        s.set("a", "pw").unwrap();
        assert_eq!(s.get("a").unwrap().as_deref(), Some("pw"));
        s.delete("a").unwrap();
        s.delete("a").unwrap();
        assert_eq!(s.get("a").unwrap(), None);
    }

    /// Writes `map` the way releases up to 0.3.0 did.
    fn write_legacy(dir: &Path, map: &BTreeMap<&str, &str>) {
        use aes_gcm::aead::{AeadCore, OsRng};
        let plain = serde_json::to_vec(map).unwrap();
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let body = Aes256Gcm::new(&KEY.into())
            .encrypt(&nonce, plain.as_slice())
            .unwrap();
        let mut raw = vec![VERSION];
        raw.extend_from_slice(&nonce);
        raw.extend_from_slice(&body);
        fs::write(dir.join(LEGACY_CREDENTIALS_FILE), B64.encode(raw)).unwrap();
    }

    /// Users who update from a release with `credentials.enc` keep every saved password, and the
    /// file with the built-in key is gone afterwards.
    #[test]
    fn migration_moves_every_entry_and_removes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        write_legacy(
            dir.path(),
            &BTreeMap::from([("conn:a:password", "pw-a"), ("ai:api_key", "sk-1")]),
        );
        let s = MemoryStore::new();
        migrate_legacy_file(dir.path(), &s).unwrap();
        assert_eq!(s.get("conn:a:password").unwrap().as_deref(), Some("pw-a"));
        assert_eq!(s.get("ai:api_key").unwrap().as_deref(), Some("sk-1"));
        assert!(!dir.path().join(LEGACY_CREDENTIALS_FILE).exists());
        // Later starts find no file and change nothing.
        migrate_legacy_file(dir.path(), &s).unwrap();
        assert_eq!(s.get("conn:a:password").unwrap().as_deref(), Some("pw-a"));
    }

    /// A secret already in the store is newer than the file copy (an earlier migration stopped
    /// part way and the user changed it since); the old file value must not bring it back.
    #[test]
    fn migration_keeps_entries_the_store_already_has() {
        let dir = tempfile::tempdir().unwrap();
        write_legacy(dir.path(), &BTreeMap::from([("conn:a:password", "old")]));
        let s = MemoryStore::new();
        s.set("conn:a:password", "new").unwrap();
        migrate_legacy_file(dir.path(), &s).unwrap();
        assert_eq!(s.get("conn:a:password").unwrap().as_deref(), Some("new"));
    }

    struct FailingStore;
    impl SecretStore for FailingStore {
        fn get(&self, _: &str) -> Result<Option<String>, AppError> {
            Ok(None)
        }
        fn set(&self, _: &str, _: &str) -> Result<(), AppError> {
            Err(secret_err("locked"))
        }
        fn delete(&self, _: &str) -> Result<(), AppError> {
            Ok(())
        }
    }

    /// When the credential store refuses a write (locked, no Secret Service), the file is the only
    /// copy of the passwords: it must stay so the next start can try again.
    #[test]
    fn failed_migration_keeps_the_file() {
        let dir = tempfile::tempdir().unwrap();
        write_legacy(dir.path(), &BTreeMap::from([("conn:a:password", "pw")]));
        let e = migrate_legacy_file(dir.path(), &FailingStore).unwrap_err();
        assert_eq!(e.code, ErrorCode::SecretStore);
        assert!(dir.path().join(LEGACY_CREDENTIALS_FILE).exists());
    }

    /// A damaged file must fail loudly and stay untouched; reading it as empty would remove it
    /// together with every password in it.
    #[test]
    fn damaged_legacy_file_is_an_error_and_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LEGACY_CREDENTIALS_FILE);
        fs::write(&path, "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRo=").unwrap();
        let err = migrate_legacy_file(dir.path(), &MemoryStore::new()).unwrap_err();
        assert_eq!(err.code, ErrorCode::SecretStore);
        // The user can only recover by removing the file, so the error must name it.
        assert!(err.message.contains(&path.display().to_string()));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRo="
        );
    }
}
