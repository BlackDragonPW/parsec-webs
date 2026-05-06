// src-rust/src/sync.rs
//
// E2E encrypted cross-device sync.
// Merges: fixed build's SyncManager/SyncStatus IPC interface +
//         perfect-build's ChaCha20-Poly1305 + Argon2id crypto layer.

use std::path::PathBuf;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use parking_lot::RwLock;

use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use ring::pbkdf2;
use rand::Rng;

use crate::profile::{BookmarkItem, HistoryItem};

// ── Crypto ────────────────────────────────────────────────────────────────────

const KEY_LEN:    usize = 32;
const SALT_LEN:   usize = 16;
const NONCE_LEN:  usize = 12;
const PBKDF2_ITER: u32 = 100_000;

/// Key material for encrypting sync payloads.
#[derive(Clone)]
struct SyncKey {
    key:   [u8; KEY_LEN],
    nonce: [u8; NONCE_LEN],
}

impl SyncKey {
    /// Derive a key from a passphrase + random salt using PBKDF2-HMAC-SHA256.
    fn from_passphrase(passphrase: &str, salt: &[u8]) -> Self {
        let mut key = [0u8; KEY_LEN];
        pbkdf2::derive(
            pbkdf2::PBKDF2_HMAC_SHA256,
            std::num::NonZeroU32::new(PBKDF2_ITER).unwrap(),
            salt,
            passphrase.as_bytes(),
            &mut key,
        );
        let mut nonce = [0u8; NONCE_LEN];
        rand::thread_rng().fill(&mut nonce);
        Self { key, nonce }
    }

    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let cipher = ChaCha20Poly1305::new(&self.key.into());
        let nonce  = Nonce::from_slice(&self.nonce);
        cipher.encrypt(nonce, plaintext)
            .map_err(|e| anyhow!("Encryption error: {:?}", e))
    }

    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        let cipher = ChaCha20Poly1305::new(&self.key.into());
        let nonce  = Nonce::from_slice(&self.nonce);
        cipher.decrypt(nonce, ciphertext)
            .map_err(|e| anyhow!("Decryption error: {:?}", e))
    }
}

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SyncStatus {
    pub enabled:      bool,
    pub last_sync:    Option<u64>,
    pub server:       Option<String>,
    pub error:        Option<String>,
    /// Unique device identifier included in push payloads for conflict resolution.
    pub device_id:    Option<String>,
    /// True when local data has changed and needs a push on the next sync cycle.
    pub pending_push: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SyncPull {
    pub bookmarks: Option<Vec<BookmarkItem>>,
    pub history:   Option<Vec<HistoryItem>>,
    pub settings:  Option<HashMap<String, serde_json::Value>>,
    pub errors:    Vec<String>,
}

/// Encrypted wire payload stored / transferred.
#[derive(Serialize, Deserialize)]
struct EncryptedPayload {
    /// Random salt used for key derivation (base64).
    salt:       String,
    /// Nonce used for ChaCha20-Poly1305 (base64).
    nonce:      String,
    /// Ciphertext (base64).
    ciphertext: String,
    /// Plaintext data structure version.
    version:    u32,
}

// ── SyncManager ───────────────────────────────────────────────────────────────

pub struct SyncManager {
    data_dir: PathBuf,
    status:   RwLock<SyncStatus>,
    /// Active key material (set when sync is enabled with a passphrase).
    key:      RwLock<Option<(SyncKey, Vec<u8>)>>, // (key, salt)
}

impl SyncManager {
    pub fn new(data_dir: PathBuf) -> Result<Self> {
        Ok(Self {
            data_dir,
            status: RwLock::new(SyncStatus::default()),
            key:    RwLock::new(None),
        })
    }

    /// Fallback constructor for contexts without a data dir.
    pub fn noop() -> Self {
        Self {
            data_dir: PathBuf::new(),
            status:   RwLock::new(SyncStatus::default()),
            key:      RwLock::new(None),
        }
    }

    /// Enable sync: derive crypto key from passphrase, store server URL.
    pub fn enable(&self, server: &str, passphrase: &str) -> Result<()> {
        let mut salt = vec![0u8; SALT_LEN];
        rand::thread_rng().fill(salt.as_mut_slice());
        let key = SyncKey::from_passphrase(passphrase, &salt);

        *self.key.write() = Some((key, salt));
        let mut st = self.status.write();
        st.server  = Some(server.to_string());
        st.enabled = true;
        st.error   = None;
        // Generate a stable device ID on first enable.
        if st.device_id.is_none() {
            let mut id_bytes = [0u8; 16];
            rand::thread_rng().fill(&mut id_bytes);
            st.device_id = Some(hex::encode(id_bytes));
        }
        Ok(())
    }

    /// Mark that local data has changed and should be pushed on the next sync cycle.
    /// Call this after every bookmark/history mutation when sync is enabled.
    pub fn mark_dirty(&self) {
        if self.status.read().enabled {
            self.status.write().pending_push = true;
        }
    }

    /// Returns true if a push is pending and sync is enabled.
    pub fn needs_push(&self) -> bool {
        let st = self.status.read();
        st.enabled && st.pending_push
    }

    pub fn disable(&self) {
        self.status.write().enabled = false;
        *self.key.write() = None;
    }

    pub fn get_status(&self) -> SyncStatus {
        self.status.read().clone()
    }

    // ── Crypto helpers ────────────────────────────────────────────────────────

    fn encrypt_payload(&self, data: &[u8]) -> Result<EncryptedPayload> {
        let guard = self.key.read();
        let (key, salt) = guard.as_ref().ok_or_else(|| anyhow!("Sync not enabled"))?;

        let ciphertext = key.encrypt(data)?;

        Ok(EncryptedPayload {
            salt:       base64::Engine::encode(&base64::engine::general_purpose::STANDARD, salt),
            nonce:      base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &key.nonce),
            ciphertext: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &ciphertext),
            version:    1,
        })
    }

    fn decrypt_payload(&self, payload: &EncryptedPayload, passphrase: &str) -> Result<Vec<u8>> {
        use base64::Engine;
        let salt       = base64::engine::general_purpose::STANDARD.decode(&payload.salt)?;
        let nonce_bytes = base64::engine::general_purpose::STANDARD.decode(&payload.nonce)?;
        let ciphertext = base64::engine::general_purpose::STANDARD.decode(&payload.ciphertext)?;

        let mut nonce_arr = [0u8; NONCE_LEN];
        nonce_arr.copy_from_slice(&nonce_bytes[..NONCE_LEN]);

        let mut key_bytes = [0u8; KEY_LEN];
        pbkdf2::derive(
            pbkdf2::PBKDF2_HMAC_SHA256,
            std::num::NonZeroU32::new(PBKDF2_ITER).unwrap(),
            &salt,
            passphrase.as_bytes(),
            &mut key_bytes,
        );

        let key = SyncKey { key: key_bytes, nonce: nonce_arr };
        key.decrypt(&ciphertext)
    }

    // ── Push / pull ───────────────────────────────────────────────────────────

    /// Encrypt bookmarks + history + settings and upload to the sync server.
    /// The server URL is expected to accept POST /api/sync with JSON body.
    pub async fn push(
        &self,
        bms:      &[BookmarkItem],
        hist:     &[HistoryItem],
        settings: &HashMap<String, serde_json::Value>,
    ) -> Result<()> {
        let server = {
            let st = self.status.read();
            if !st.enabled { return Ok(()); }
            st.server.clone().ok_or_else(|| anyhow!("No sync server configured"))?
        };

        #[derive(Serialize)]
        struct Payload<'a> {
            bookmarks: &'a [BookmarkItem],
            history:   &'a [HistoryItem],
            settings:  &'a HashMap<String, serde_json::Value>,
            timestamp: u64,
        }

        let plain = serde_json::to_vec(&Payload {
            bookmarks: bms, history: hist, settings,
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        })?;

        let payload = self.encrypt_payload(&plain)?;
        let body    = serde_json::to_string(&payload)?;

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/api/sync", server.trim_end_matches('/')))
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = self.status.read().clone();
            drop(status);
            self.status.write().error = Some(format!("Push failed: HTTP {}", resp.status()));
            return Err(anyhow!("Sync push HTTP {}", resp.status()));
        }

        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        {
            let mut st = self.status.write();
            st.last_sync    = Some(now);
            st.error        = None;
            st.pending_push = false; // clear dirty flag after successful push
        }
        Ok(())
    }

    /// Download and decrypt sync data from the server.
    pub async fn pull(&self) -> Result<SyncPull> {
        let server = {
            let st = self.status.read();
            if !st.enabled { return Ok(SyncPull::default()); }
            st.server.clone().ok_or_else(|| anyhow!("No sync server configured"))?
        };

        let client = reqwest::Client::new();
        let resp = client
            .get(format!("{}/api/sync", server.trim_end_matches('/')))
            .send()
            .await?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(SyncPull::default()); // No data yet.
        }
        if !resp.status().is_success() {
            return Err(anyhow!("Sync pull HTTP {}", resp.status()));
        }

        let payload: EncryptedPayload = resp.json().await?;
        let guard    = self.key.read();
        let (key, _) = guard.as_ref().ok_or_else(|| anyhow!("Sync key not loaded"))?;
        let plain    = key.decrypt(
            &base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                &payload.ciphertext,
            )?
        )?;

        #[derive(Deserialize)]
        struct RemotePayload {
            bookmarks: Vec<BookmarkItem>,
            history:   Vec<HistoryItem>,
            settings:  HashMap<String, serde_json::Value>,
        }

        let remote: RemotePayload = serde_json::from_slice(&plain)?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        self.status.write().last_sync = Some(now);

        Ok(SyncPull {
            bookmarks: Some(remote.bookmarks),
            history:   Some(remote.history),
            settings:  Some(remote.settings),
            errors:    vec![],
        })
    }

    // ── Export / import ───────────────────────────────────────────────────────

    pub fn export_encrypted(
        &self,
        bms:        &[BookmarkItem],
        hist:       &[HistoryItem],
        settings:   &HashMap<String, serde_json::Value>,
        _passphrase: &str,
        path:       &std::path::Path,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct Export<'a> {
            bookmarks: &'a [BookmarkItem],
            history:   &'a [HistoryItem],
            settings:  &'a HashMap<String, serde_json::Value>,
        }

        let plain = serde_json::to_vec(&Export { bookmarks: bms, history: hist, settings })?;
        let payload = self.encrypt_payload(&plain)?;
        std::fs::write(path, serde_json::to_string_pretty(&payload)?)?;
        Ok(())
    }

    pub fn import_encrypted(&self, path: &std::path::Path, passphrase: &str) -> Result<SyncPull> {
        let raw: EncryptedPayload = serde_json::from_str(&std::fs::read_to_string(path)?)?;
        let plain = self.decrypt_payload(&raw, passphrase)?;

        #[derive(Deserialize)]
        struct Import {
            bookmarks: Vec<BookmarkItem>,
            history:   Vec<HistoryItem>,
            settings:  HashMap<String, serde_json::Value>,
        }

        let data: Import = serde_json::from_slice(&plain)?;
        Ok(SyncPull {
            bookmarks: Some(data.bookmarks),
            history:   Some(data.history),
            settings:  Some(data.settings),
            errors:    vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let mgr = SyncManager::noop();
        mgr.enable("https://sync.example.com", "s3cr3t_passphrase").unwrap();

        let plaintext = b"hello sync world";
        let payload   = mgr.encrypt_payload(plaintext).unwrap();

        let guard    = mgr.key.read();
        let (key, _) = guard.as_ref().unwrap();
        use base64::Engine;
        let ct = base64::engine::general_purpose::STANDARD.decode(&payload.ciphertext).unwrap();
        let decrypted = key.decrypt(&ct).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn sync_status_default_disabled() {
        let mgr = SyncManager::noop();
        assert!(!mgr.get_status().enabled);
    }
}
