// src-tauri/src/sync.rs
//
// v1.3: Cross-device sync with end-to-end encryption.
//
// Architecture
// ────────────
// All data is encrypted client-side before leaving the device.
// The server (configurable, can be self-hosted) stores only opaque blobs.
// The server never sees plaintext.
//
// Crypto
// ──────
//   Key derivation: Argon2id(passphrase + device_salt) → 32-byte key
//   Encryption:     XChaCha20-Poly1305  (random 24-byte nonce per blob)
//   Format:         nonce(24) || ciphertext || tag(16)
//
// Protocol
// ────────
//   PUT  /sync/{user_id}/{collection}?v={version}
//        Body: encrypted blob (application/octet-stream)
//        Response: { "version": <new_version> }
//
//   GET  /sync/{user_id}/{collection}?since={version}
//        Response: { "version": <v>, "data": <b64_blob> } | 304 Not Modified
//
//   DELETE /sync/{user_id}/{collection}
//
// Collections:
//   bookmarks    — Vec<BookmarkItem>
//   history      — Vec<HistoryItem> (last 1000)
//   settings     — HashMap<String, Value>
//   extensions   — Vec<InstalledExtension> (metadata only, no source)
//   sessions     — Vec<TabSession>
//
// Each collection is a single encrypted JSON blob.
// The server is stateless; clients use a version number (u64 timestamp)
// for conflict resolution: last-write-wins per collection.
//
// Self-hosting
// ────────────
//   docker run -p 8080:8080 parsec/sync-server
//   Or point to any S3-compatible store (the client uses plain HTTP PUT/GET).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use serde::{Deserialize, Serialize};
use anyhow::{Context, Result, anyhow, bail};
use tracing::{info, warn, debug};
use reqwest::Client;
use base64::Engine as _;

use crate::profile::{BookmarkItem, HistoryItem, TabSession};

// ── Crypto ────────────────────────────────────────────────────────────────────

use chacha20poly1305::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    XChaCha20Poly1305, XNonce, Key,
};

fn derive_key(passphrase: &str, salt: &[u8; 32]) -> Key {
    use argon2::{Argon2, Params, Algorithm, Version};
    let params = Params::new(
        32_768, // m_cost: 32 MiB
        3,      // t_cost: 3 iterations
        1,      // p_cost: 1 lane
        Some(32),
    ).expect("argon2 params");
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    argon2.hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .expect("argon2 hash");
    Key::from(key)
}

fn encrypt(key: &Key, plaintext: &[u8]) -> Vec<u8> {
    let cipher = XChaCha20Poly1305::new(key);
    let nonce  = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ct     = cipher.encrypt(&nonce, plaintext).expect("encrypt");
    let mut out = Vec::with_capacity(24 + ct.len());
    out.extend_from_slice(nonce.as_slice());
    out.extend_from_slice(&ct);
    out
}

fn decrypt(key: &Key, ciphertext: &[u8]) -> Result<Vec<u8>> {
    if ciphertext.len() < 24 {
        bail!("Ciphertext too short");
    }
    let (nonce_bytes, ct) = ciphertext.split_at(24);
    let nonce  = XNonce::from_slice(nonce_bytes);
    let cipher = XChaCha20Poly1305::new(key);
    cipher.decrypt(nonce, ct)
        .map_err(|_| anyhow!("Decryption failed — wrong passphrase or corrupted data"))
}

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    pub enabled:     bool,
    pub server_url:  String,
    pub user_id:     String,
    /// Base64-encoded 32-byte salt (stored locally, never synced)
    pub device_salt: String,
    /// Last known version per collection (local cache of server version)
    pub versions:    HashMap<String, u64>,
}

impl Default for SyncConfig {
    fn default() -> Self {
        let mut salt = [0u8; 32];
        getrandom::getrandom(&mut salt).ok();
        Self {
            enabled:    false,
            server_url: "https://sync.parsec.os".into(),
            user_id:    uuid_v4(),
            device_salt: base64::engine::general_purpose::STANDARD.encode(salt),
            versions:   HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncStatus {
    pub last_sync:    Option<u64>,    // unix ms
    pub collections:  HashMap<String, CollectionStatus>,
    pub error:        Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionStatus {
    pub version:       u64,
    pub last_modified: u64,  // unix ms
    pub item_count:    usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ServerResponse {
    version: u64,
    data:    Option<String>,  // base64 blob (only on GET)
    error:   Option<String>,
}

// ── SyncManager ───────────────────────────────────────────────────────────────

pub struct SyncManager {
    config:    Arc<Mutex<SyncConfig>>,
    status:    Arc<Mutex<SyncStatus>>,
    client:    Client,
    data_dir:  PathBuf,
}

impl SyncManager {
    pub fn new(data_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&data_dir)?;

        let config_path = data_dir.join("sync_config.json");
        let config = if config_path.exists() {
            let text = std::fs::read_to_string(&config_path)?;
            serde_json::from_str::<SyncConfig>(&text).unwrap_or_default()
        } else {
            SyncConfig::default()
        };

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("ParsecWeb/1.3 sync")
            .build()?;

        Ok(Self {
            config: Arc::new(Mutex::new(config)),
            status: Arc::new(Mutex::new(SyncStatus {
                last_sync: None,
                collections: HashMap::new(),
                error: None,
            })),
            client,
            data_dir,
        })
    }

    pub fn get_config(&self) -> SyncConfig {
        self.config.lock().unwrap().clone()
    }

    pub fn get_status(&self) -> SyncStatus {
        self.status.lock().unwrap().clone()
    }

    pub fn configure(
        &self,
        server_url: &str,
        user_id: &str,
        enabled: bool,
    ) -> Result<()> {
        let mut cfg = self.config.lock().unwrap();
        cfg.server_url = server_url.trim_end_matches('/').to_string();
        cfg.user_id    = user_id.to_string();
        cfg.enabled    = enabled;
        self.save_config(&cfg)
    }

    pub fn set_enabled(&self, enabled: bool) {
        let mut cfg = self.config.lock().unwrap();
        cfg.enabled = enabled;
        self.save_config(&cfg).ok();
    }

    fn save_config(&self, cfg: &SyncConfig) -> Result<()> {
        let path = self.data_dir.join("sync_config.json");
        let text = serde_json::to_string_pretty(cfg)?;
        std::fs::write(&path, text)?;
        Ok(())
    }

    // ── Encryption helpers ──────────────────────────────────────────────────

    fn get_key(&self, passphrase: &str) -> Result<Key> {
        let cfg   = self.config.lock().unwrap();
        let salt_b64 = cfg.device_salt.clone();
        drop(cfg);

        let salt_bytes = base64::engine::general_purpose::STANDARD
            .decode(&salt_b64)
            .context("decode salt")?;
        let mut salt = [0u8; 32];
        if salt_bytes.len() != 32 { bail!("Invalid salt length"); }
        salt.copy_from_slice(&salt_bytes);
        Ok(derive_key(passphrase, &salt))
    }

    // ── Single collection push/pull ─────────────────────────────────────────

    pub async fn push_collection(
        &self,
        collection: &str,
        data: &[u8],
        passphrase: &str,
    ) -> Result<u64> {
        let cfg = self.config.lock().unwrap().clone();
        if !cfg.enabled { bail!("Sync disabled"); }

        let key        = self.get_key(passphrase)?;
        let ciphertext = encrypt(&key, data);
        let version    = crate::unix_ms();

        let url = format!("{}/sync/{}/{collection}?v={version}",
            cfg.server_url, cfg.user_id);

        let resp = self.client.put(&url)
            .header("Content-Type", "application/octet-stream")
            .body(ciphertext)
            .send().await
            .context("sync push request")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body   = resp.text().await.unwrap_or_default();
            bail!("Sync push HTTP {status}: {body}");
        }

        // Update local version cache
        {
            let mut cfg_mut = self.config.lock().unwrap();
            cfg_mut.versions.insert(collection.to_string(), version);
            self.save_config(&cfg_mut).ok();
        }

        info!("Sync: pushed '{collection}' ({} bytes plaintext, v{version})", data.len());
        Ok(version)
    }

    pub async fn pull_collection(
        &self,
        collection: &str,
        passphrase: &str,
    ) -> Result<Option<Vec<u8>>> {
        let cfg = self.config.lock().unwrap().clone();
        if !cfg.enabled { bail!("Sync disabled"); }

        let since = cfg.versions.get(collection).copied().unwrap_or(0);
        let url   = format!("{}/sync/{}/{collection}?since={since}",
            cfg.server_url, cfg.user_id);

        let resp = self.client.get(&url).send().await
            .context("sync pull request")?;

        if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
            debug!("Sync: '{collection}' up to date");
            return Ok(None);
        }
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            debug!("Sync: '{collection}' not on server yet");
            return Ok(None);
        }
        if !resp.status().is_success() {
            let s = resp.status();
            bail!("Sync pull HTTP {s}");
        }

        let server_resp: ServerResponse = resp.json().await
            .context("sync pull parse")?;

        if let Some(data_b64) = server_resp.data {
            let ciphertext = base64::engine::general_purpose::STANDARD
                .decode(&data_b64)
                .context("decode sync blob")?;
            let key       = self.get_key(passphrase)?;
            let plaintext = decrypt(&key, &ciphertext)?;

            // Update version
            {
                let mut cfg_mut = self.config.lock().unwrap();
                cfg_mut.versions.insert(collection.to_string(), server_resp.version);
                self.save_config(&cfg_mut).ok();
            }

            info!("Sync: pulled '{collection}' ({} bytes, v{})",
                plaintext.len(), server_resp.version);
            Ok(Some(plaintext))
        } else {
            Ok(None)
        }
    }

    // ── Full sync ───────────────────────────────────────────────────────────

    /// Push all profile data to the server.
    pub async fn push_all(
        &self,
        bookmarks: &[BookmarkItem],
        history:   &[HistoryItem],
        settings:  &HashMap<String, serde_json::Value>,
        sessions:  &[TabSession],
        passphrase: &str,
    ) -> Result<SyncSummary> {
        let mut summary = SyncSummary::default();

        // Push bookmarks
        match serde_json::to_vec(bookmarks) {
            Ok(data) => match self.push_collection("bookmarks", &data, passphrase).await {
                Ok(v) => { summary.pushed.push(format!("bookmarks ({})", bookmarks.len())); summary.versions.insert("bookmarks".into(), v); }
                Err(e) => { summary.errors.push(format!("bookmarks: {e}")); }
            }
            Err(e) => summary.errors.push(format!("bookmarks serialize: {e}")),
        }

        // Push history (last 1000 only — don't sync everything)
        let recent_history: Vec<&HistoryItem> = history.iter().take(1000).collect();
        match serde_json::to_vec(&recent_history) {
            Ok(data) => match self.push_collection("history", &data, passphrase).await {
                Ok(v) => { summary.pushed.push(format!("history ({})", recent_history.len())); summary.versions.insert("history".into(), v); }
                Err(e) => { summary.errors.push(format!("history: {e}")); }
            }
            Err(e) => summary.errors.push(format!("history serialize: {e}")),
        }

        // Push settings
        match serde_json::to_vec(settings) {
            Ok(data) => match self.push_collection("settings", &data, passphrase).await {
                Ok(v) => { summary.pushed.push("settings".into()); summary.versions.insert("settings".into(), v); }
                Err(e) => { summary.errors.push(format!("settings: {e}")); }
            }
            Err(e) => summary.errors.push(format!("settings serialize: {e}")),
        }

        // Push sessions
        match serde_json::to_vec(sessions) {
            Ok(data) => match self.push_collection("sessions", &data, passphrase).await {
                Ok(v) => { summary.pushed.push(format!("sessions ({})", sessions.len())); summary.versions.insert("sessions".into(), v); }
                Err(e) => { summary.errors.push(format!("sessions: {e}")); }
            }
            Err(e) => summary.errors.push(format!("sessions serialize: {e}")),
        }

        // Update status
        {
            let mut status = self.status.lock().unwrap();
            status.last_sync = Some(crate::unix_ms());
            if summary.errors.is_empty() {
                status.error = None;
            } else {
                status.error = Some(summary.errors.join("; "));
            }
        }

        info!("Sync push complete: {:?}", summary.pushed);
        Ok(summary)
    }

    /// Pull all collections from the server and return updated data.
    pub async fn pull_all(
        &self,
        passphrase: &str,
    ) -> Result<SyncPullResult> {
        let mut result = SyncPullResult::default();

        // Pull bookmarks
        match self.pull_collection("bookmarks", passphrase).await {
            Ok(Some(data)) => {
                match serde_json::from_slice::<Vec<BookmarkItem>>(&data) {
                    Ok(bms) => result.bookmarks = Some(bms),
                    Err(e)  => result.errors.push(format!("bookmarks parse: {e}")),
                }
            }
            Ok(None) => {}
            Err(e)   => result.errors.push(format!("bookmarks pull: {e}")),
        }

        // Pull history
        match self.pull_collection("history", passphrase).await {
            Ok(Some(data)) => {
                match serde_json::from_slice::<Vec<HistoryItem>>(&data) {
                    Ok(h)  => result.history = Some(h),
                    Err(e) => result.errors.push(format!("history parse: {e}")),
                }
            }
            Ok(None) => {}
            Err(e)   => result.errors.push(format!("history pull: {e}")),
        }

        // Pull settings
        match self.pull_collection("settings", passphrase).await {
            Ok(Some(data)) => {
                match serde_json::from_slice::<HashMap<String, serde_json::Value>>(&data) {
                    Ok(s)  => result.settings = Some(s),
                    Err(e) => result.errors.push(format!("settings parse: {e}")),
                }
            }
            Ok(None) => {}
            Err(e)   => result.errors.push(format!("settings pull: {e}")),
        }

        // Pull sessions
        match self.pull_collection("sessions", passphrase).await {
            Ok(Some(data)) => {
                match serde_json::from_slice::<Vec<TabSession>>(&data) {
                    Ok(s)  => result.sessions = Some(s),
                    Err(e) => result.errors.push(format!("sessions parse: {e}")),
                }
            }
            Ok(None) => {}
            Err(e)   => result.errors.push(format!("sessions pull: {e}")),
        }

        // Update status
        {
            let mut status = self.status.lock().unwrap();
            status.last_sync = Some(crate::unix_ms());
            if result.errors.is_empty() {
                status.error = None;
            } else {
                status.error = Some(result.errors.join("; "));
            }
        }

        info!("Sync pull complete: bookmarks={}, history={}, settings={}, sessions={}",
            result.bookmarks.as_ref().map(|v| v.len()).unwrap_or(0),
            result.history.as_ref().map(|v| v.len()).unwrap_or(0),
            result.settings.is_some(),
            result.sessions.as_ref().map(|v| v.len()).unwrap_or(0),
        );

        Ok(result)
    }

    // ── Account management ──────────────────────────────────────────────────

    /// Register a new account on the sync server.
    /// Returns the user_id to save locally.
    pub async fn register_account(&self, email: &str, passphrase: &str) -> Result<String> {
        let cfg = self.config.lock().unwrap().clone();
        let url = format!("{}/auth/register", cfg.server_url);

        let body = serde_json::json!({
            "email":    email,
            "user_id":  cfg.user_id,
            // We send a hashed token, not the raw passphrase
            "token":    password_token(passphrase, &cfg.user_id),
        });

        let resp = self.client.post(&url).json(&body).send().await
            .context("register request")?;

        if !resp.status().is_success() {
            let s = resp.status();
            let b = resp.text().await.unwrap_or_default();
            bail!("Register failed HTTP {s}: {b}");
        }

        let result: serde_json::Value = resp.json().await?;
        let user_id = result["user_id"].as_str()
            .unwrap_or(&cfg.user_id).to_string();

        info!("Sync: registered account for {email} (id: {user_id})");
        Ok(user_id)
    }

    /// Verify credentials with the server.
    pub async fn verify_credentials(&self, passphrase: &str) -> Result<bool> {
        let cfg = self.config.lock().unwrap().clone();
        let url = format!("{}/auth/verify", cfg.server_url);

        let body = serde_json::json!({
            "user_id": cfg.user_id,
            "token":   password_token(passphrase, &cfg.user_id),
        });

        let resp = self.client.post(&url).json(&body).send().await
            .context("verify request")?;
        Ok(resp.status().is_success())
    }

    // ── File-based sync (Dropbox / iCloud / NFS / USB) ──────────────────────

    /// Export all data as an encrypted file for offline sync.
    pub fn export_encrypted(
        &self,
        bookmarks: &[BookmarkItem],
        history:   &[HistoryItem],
        settings:  &HashMap<String, serde_json::Value>,
        passphrase: &str,
        output_path: &Path,
    ) -> Result<()> {
        let payload = serde_json::json!({
            "bookmarks": bookmarks,
            "history":   &history[..history.len().min(1000)],
            "settings":  settings,
            "version":   crate::unix_ms(),
        });
        let data    = serde_json::to_vec(&payload)?;
        let key     = self.get_key(passphrase)?;
        let ct      = encrypt(&key, &data);
        std::fs::write(output_path, ct).context("write export file")?;
        info!("Sync: exported encrypted file to {:?}", output_path);
        Ok(())
    }

    /// Import from an encrypted file (Dropbox / USB sync).
    pub fn import_encrypted(
        &self,
        input_path: &Path,
        passphrase: &str,
    ) -> Result<SyncPullResult> {
        let ct    = std::fs::read(input_path).context("read import file")?;
        let key   = self.get_key(passphrase)?;
        let data  = decrypt(&key, &ct)?;
        let json: serde_json::Value = serde_json::from_slice(&data)?;

        let mut result = SyncPullResult::default();

        if let Some(bms) = json.get("bookmarks") {
            result.bookmarks = serde_json::from_value(bms.clone()).ok();
        }
        if let Some(h) = json.get("history") {
            result.history = serde_json::from_value(h.clone()).ok();
        }
        if let Some(s) = json.get("settings") {
            result.settings = serde_json::from_value(s.clone()).ok();
        }
        info!("Sync: imported encrypted file from {:?}", input_path);
        Ok(result)
    }
}

// ── Result types ──────────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SyncSummary {
    pub pushed:   Vec<String>,
    pub errors:   Vec<String>,
    pub versions: HashMap<String, u64>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SyncPullResult {
    pub bookmarks: Option<Vec<BookmarkItem>>,
    pub history:   Option<Vec<HistoryItem>>,
    pub settings:  Option<HashMap<String, serde_json::Value>>,
    pub sessions:  Option<Vec<TabSession>>,
    pub errors:    Vec<String>,
}

// ── Utilities ─────────────────────────────────────────────────────────────────

/// Derive a server authentication token from passphrase + user_id.
/// This is what we send to the server for auth — NOT the encryption key.
/// The server stores a bcrypt hash of this token (never sees the passphrase).
///
/// Uses HMAC-SHA256(key=user_id, msg=passphrase) so the token is:
///   - Deterministic: same inputs → same 64-char hex token
///   - One-way: cannot recover passphrase from token
///   - Distinct from the encryption key: encryption uses Argon2id + device salt
fn password_token(passphrase: &str, user_id: &str) -> String {
    use ring::hmac;
    let key = hmac::Key::new(hmac::HMAC_SHA256, user_id.as_bytes());
    let tag  = hmac::sign(&key, passphrase.as_bytes());
    tag.as_ref().iter().map(|b| format!("{:02x}", b)).collect()
}

fn uuid_v4() -> String {
    let mut b = [0u8; 16];
    getrandom::getrandom(&mut b).ok();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    format!("{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0],b[1],b[2],b[3],b[4],b[5],b[6],b[7],b[8],b[9],b[10],b[11],b[12],b[13],b[14],b[15])
}

// ── Sync server reference implementation (doc comment) ───────────────────────
//
// The sync server is intentionally simple — a key-value store over HTTP.
// Any server that implements these 3 endpoints works:
//
// ```
// PUT  /sync/:user_id/:collection?v=<timestamp>
//   Body: raw bytes (encrypted blob)
//   Response: { "version": <timestamp> }
//
// GET  /sync/:user_id/:collection?since=<version>
//   Response 200: { "version": <v>, "data": "<base64 blob>" }
//   Response 304: (empty — client is up to date)
//   Response 404: (collection doesn't exist yet)
//
// POST /auth/register  { "email", "user_id", "token" }
//   Response 200: { "user_id": "..." }
//
// POST /auth/verify    { "user_id", "token" }
//   Response 200: (success)
//   Response 401: (wrong credentials)
// ```
//
// A minimal Node.js server (< 100 lines) is at:
//   https://github.com/parsec-web/sync-server
//
// Or use Cloudflare Workers KV / AWS Lambda + DynamoDB / Supabase.
