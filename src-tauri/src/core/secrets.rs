//! Password storage. The app keeps passwords in an encrypted file in the data dir (`credentials.enc`),
//! like DBeaver's `credentials-config.json`; tests and the dev bridge use memory only.
//!
//! The key is built into the app, so the encryption only keeps passwords out of plain sight (backups,
//! grep, screen shares). Anyone with the file and the app binary can decrypt it. The file is readable
//! by the user only (0600 on Unix).

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;

use crate::core::error::{AppError, ErrorCode};
use crate::core::store::write_atomic;

pub const CREDENTIALS_FILE: &str = "credentials.enc";

/// Fixed on purpose: changing it makes every saved password unreadable.
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
        "Could not read or write the password file",
    )
    .with_detail(e.to_string())
}

/// The whole map is read and written on every call; the file holds one entry per connection.
pub struct FileStore {
    path: PathBuf,
    /// Serializes read-modify-write so two saves never drop each other's entry.
    write: Mutex<()>,
}

impl FileStore {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            path: data_dir.into().join(CREDENTIALS_FILE),
            write: Mutex::new(()),
        }
    }

    /// A missing file is an empty store. A file that does not decrypt is an error, never an empty
    /// store: treating it as empty would let the next save wipe every other password.
    fn load(&self) -> Result<BTreeMap<String, String>, AppError> {
        let text = match fs::read_to_string(&self.path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(e) => return Err(secret_err(e)),
        };
        // No decode or parse detail: it could quote the decrypted content.
        let damaged = || {
            secret_err(format!(
                "{} is damaged. Remove it and enter your connection passwords again.",
                self.path.display()
            ))
        };
        let raw = B64.decode(text.trim()).map_err(|_| damaged())?;
        if raw.len() < 1 + NONCE_LEN || raw[0] != VERSION {
            return Err(damaged());
        }
        let (nonce, body) = raw[1..].split_at(NONCE_LEN);
        let plain = Aes256Gcm::new(&KEY.into())
            .decrypt(Nonce::from_slice(nonce), body)
            .map_err(|_| damaged())?;
        serde_json::from_slice(&plain).map_err(|_| damaged())
    }

    fn save(&self, map: &BTreeMap<String, String>) -> Result<(), AppError> {
        let plain = serde_json::to_vec(map).map_err(secret_err)?;
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let body = Aes256Gcm::new(&KEY.into())
            .encrypt(&nonce, plain.as_slice())
            .map_err(secret_err)?;
        let mut raw = Vec::with_capacity(1 + NONCE_LEN + body.len());
        raw.push(VERSION);
        raw.extend_from_slice(&nonce);
        raw.extend_from_slice(&body);
        write_atomic(&self.path, B64.encode(raw).as_bytes(), true).map_err(secret_err)
    }
}

impl SecretStore for FileStore {
    fn get(&self, account: &str) -> Result<Option<String>, AppError> {
        Ok(self.load()?.remove(account))
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), AppError> {
        let _w = self.write.lock().unwrap_or_else(|e| e.into_inner());
        let mut map = self.load()?;
        map.insert(account.to_string(), secret.to_string());
        self.save(&map)
    }

    fn delete(&self, account: &str) -> Result<(), AppError> {
        let _w = self.write.lock().unwrap_or_else(|e| e.into_inner());
        let mut map = self.load()?;
        if map.remove(account).is_none() {
            return Ok(());
        }
        self.save(&map)
    }
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

    /// Passwords must survive an app restart: a second store on the same dir reads what the first wrote.
    #[test]
    fn file_store_persists_across_restarts() {
        let dir = tempfile::tempdir().unwrap();
        let s = FileStore::new(dir.path());
        assert_eq!(s.get("a").unwrap(), None);
        s.set("a", "pw-a").unwrap();
        s.set("b", "pw-b").unwrap();
        s.set("a", "pw-a2").unwrap();
        s.delete("b").unwrap();
        s.delete("b").unwrap();

        let reopened = FileStore::new(dir.path());
        assert_eq!(reopened.get("a").unwrap().as_deref(), Some("pw-a2"));
        assert_eq!(reopened.get("b").unwrap(), None);
    }

    /// The point of the file over plain JSON: the password text is not on disk in the clear.
    #[test]
    fn file_store_never_writes_the_password_in_plain_text() {
        let dir = tempfile::tempdir().unwrap();
        let s = FileStore::new(dir.path());
        s.set("conn:x:password", "hunter2-very-secret").unwrap();
        let raw = fs::read(dir.path().join(CREDENTIALS_FILE)).unwrap();
        let text = String::from_utf8_lossy(&raw);
        assert!(!text.contains("hunter2"), "{text}");
        assert!(
            !text.contains("conn:x"),
            "account names are encrypted too: {text}"
        );
    }

    /// A damaged file must fail loudly and stay untouched; reading it as empty would let the next
    /// save delete every other stored password.
    #[test]
    fn damaged_file_is_an_error_and_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CREDENTIALS_FILE);
        fs::write(&path, "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRo=").unwrap();
        let s = FileStore::new(dir.path());
        let err = s.get("a").unwrap_err();
        assert_eq!(err.code, ErrorCode::SecretStore);
        // The user can only recover by removing the file, so the error must name it.
        assert!(err.detail.unwrap().contains(&path.display().to_string()));
        assert_eq!(s.set("a", "pw").unwrap_err().code, ErrorCode::SecretStore);
        assert_eq!(s.delete("a").unwrap_err().code, ErrorCode::SecretStore);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRo="
        );
    }

    #[cfg(unix)]
    #[test]
    fn file_store_file_is_private_to_the_user() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let s = FileStore::new(dir.path());
        s.set("a", "pw").unwrap();
        let mode = fs::metadata(dir.path().join(CREDENTIALS_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
