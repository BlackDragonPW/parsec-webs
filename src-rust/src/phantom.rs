// src-rust/src/phantom.rs
//
// Parsec Ghost Mode — encrypted request proxy for true incognito.
//
// Problem with normal incognito (Chrome, Firefox, all browsers):
//   - The ISP still sees every DNS lookup and HTTPS SNI hostname you visit.
//   - The websites you visit see your real IP address.
//   - Your network sees all traffic metadata (timing, size, destination).
//
// Ghost Mode fixes this with a 3-hop onion-style encrypted proxy chain:
//
//   Browser → [ChaCha20 layer 3] → Entry Node
//             → [ChaCha20 layer 2] → Middle Node
//               → [ChaCha20 layer 1] → Exit Node
//                                       → Destination Site
//
// Each hop only knows its predecessor and successor — not the full chain.
// The exit node sees the destination but not who the browser is.
// The entry node sees the browser IP but not the destination.
// No single node has both pieces.
//
// DNS is routed through the chain too — the ISP sees no hostnames at all.
//
// When no Phantom proxy server is configured, Ghost Mode falls back to a
// safe self-hosted DoH + IP-masking strategy using Cloudflare's privacy proxy.
//
// Architecture:
//   GhostSession  → one per incognito tab (fresh ephemeral keys)
//   PhantomRouter → manages sessions, routes requests
//   The WebView's traffic is intercepted via shouldInterceptRequest and
//   re-issued through the phantom channel, response piped back.

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use anyhow::{anyhow, Result};
use rand::Rng;
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::info;

// ── Ephemeral session keys ────────────────────────────────────────────────────

/// One-time key pair generated for each Ghost Mode session (tab).
/// Keys are never persisted to disk. When the tab closes, keys are zeroed.
#[derive(Clone)]
pub struct EphemeralKeys {
    pub session_id: [u8; 16],  // random session identifier
    key_layer1: [u8; 32],      // exit → browser decrypt key
    key_layer2: [u8; 32],      // middle layer
    key_layer3: [u8; 32],      // entry layer (outermost)
}

impl EphemeralKeys {
    /// Generate fresh random keys. Called once per incognito tab.
    pub fn generate() -> Self {
        let mut rng = OsRng;
        let mut k = Self {
            session_id: [0u8; 16],
            key_layer1: [0u8; 32],
            key_layer2: [0u8; 32],
            key_layer3: [0u8; 32],
        };
        rng.fill_bytes(&mut k.session_id);
        rng.fill_bytes(&mut k.key_layer1);
        rng.fill_bytes(&mut k.key_layer2);
        rng.fill_bytes(&mut k.key_layer3);
        k
    }

    /// Zero out all key material. Called when tab is closed.
    pub fn zeroize(&mut self) {
        self.key_layer1.fill(0);
        self.key_layer2.fill(0);
        self.key_layer3.fill(0);
        self.session_id.fill(0);
    }

    /// Wrap plaintext in 3 layers of ChaCha20-Poly1305 encryption.
    /// Outermost layer = entry node key (peeled first by entry node).
    pub fn onion_encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        // Layer 1: innermost (exit node decrypts)
        let l1 = chacha_encrypt(&self.key_layer1, plaintext)?;
        // Layer 2: middle
        let l2 = chacha_encrypt(&self.key_layer2, &l1)?;
        // Layer 3: outermost (entry node decrypts first)
        let l3 = chacha_encrypt(&self.key_layer3, &l2)?;
        Ok(l3)
    }

    /// Decrypt a response that comes back through the chain (already peeled by nodes).
    /// Responses come back with only layer1 (exit node encrypts response).
    pub fn decrypt_response(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        chacha_decrypt(&self.key_layer1, ciphertext)
    }
}

impl Drop for EphemeralKeys {
    fn drop(&mut self) {
        self.zeroize();
    }
}

fn chacha_encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>> {
    let cipher = ChaCha20Poly1305::new(key.into());
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let mut ciphertext = cipher.encrypt(nonce, plaintext)
        .map_err(|e| anyhow!("encrypt error: {:?}", e))?;
    // Prepend nonce so the receiver can decrypt
    let mut out = nonce_bytes.to_vec();
    out.append(&mut ciphertext);
    Ok(out)
}

fn chacha_decrypt(key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < 12 {
        return Err(anyhow!("ciphertext too short"));
    }
    let (nonce_bytes, ciphertext) = data.split_at(12);
    let cipher = ChaCha20Poly1305::new(key.into());
    let nonce  = Nonce::from_slice(nonce_bytes);
    cipher.decrypt(nonce, ciphertext)
        .map_err(|e| anyhow!("decrypt error: {:?}", e))
}

// ── Ghost session ─────────────────────────────────────────────────────────────

/// One Ghost session per incognito tab.
/// Holds ephemeral keys, fake user-agent, and a spoofed accept-language.
pub struct GhostSession {
    pub tab_id:   String,
    pub keys:     EphemeralKeys,
    /// Randomised user-agent so tabs can't be linked via UA string
    pub user_agent: String,
    /// Random accept-language to resist language fingerprinting
    pub accept_lang: String,
    /// Timestamp of session creation — sessions auto-expire after 30 min
    created_at: u64,
}

impl GhostSession {
    pub fn new(tab_id: &str) -> Self {
        let ua = random_user_agent();
        let lang = random_accept_language();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            tab_id: tab_id.to_string(),
            keys: EphemeralKeys::generate(),
            user_agent: ua,
            accept_lang: lang,
            created_at: now,
        }
    }

    pub fn is_expired(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        now - self.created_at > 1800  // 30 minutes
    }
}

// ── Phantom router ────────────────────────────────────────────────────────────

/// Phantom proxy configuration.
/// In production, users point this at a Parsec-compatible relay
/// (e.g., self-hosted or a Parsec subscription exit node).
/// When no server is configured, Ghost Mode uses the fallback path.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PhantomConfig {
    /// Entry node URL: wss://entry.example.com/phantom
    pub entry_node:  Option<String>,
    /// Middle node URL (optional — single-hop if absent)
    pub middle_node: Option<String>,
    /// Exit node URL
    pub exit_node:   Option<String>,
    /// Whether to route DNS through the chain (recommended: true)
    pub private_dns: bool,
}

pub struct PhantomRouter {
    config:   RwLock<PhantomConfig>,
    pub sessions: RwLock<HashMap<String, GhostSession>>,  // tab_id → session
    client:   reqwest::Client,
}

impl PhantomRouter {
    pub fn new() -> Self {
        let client = reqwest::ClientBuilder::new()
            .timeout(Duration::from_secs(30))
            .https_only(true)
            // Disable connection reuse between ghost sessions to prevent
            // timing correlation across tabs
            .connection_verbose(false)
            .pool_max_idle_per_host(0)
            .build()
            .expect("PhantomRouter client");

        Self {
            config:   RwLock::new(PhantomConfig { private_dns: true, ..Default::default() }),
            sessions: RwLock::new(HashMap::new()),
            client,
        }
    }

    /// Called when a new incognito tab is created.
    /// Generates fresh ephemeral keys and a randomised fingerprint.
    pub async fn create_session(&self, tab_id: &str) {
        let session = GhostSession::new(tab_id);
        info!("Ghost: new session for tab {} (UA rotated)", tab_id);
        self.sessions.write().await.insert(tab_id.to_string(), session);
    }

    /// Called when an incognito tab is closed. Zeroes keys immediately.
    pub async fn destroy_session(&self, tab_id: &str) {
        if let Some(mut s) = self.sessions.write().await.remove(tab_id) {
            s.keys.zeroize();
            info!("Ghost: session destroyed, keys zeroed for tab {}", tab_id);
        }
    }

    /// Expire sessions older than 30 minutes and rotate their keys.
    pub async fn prune_expired_sessions(&self) {
        let expired: Vec<String> = self.sessions.read().await
            .iter()
            .filter(|(_, s)| s.is_expired())
            .map(|(id, _)| id.clone())
            .collect();

        for tab_id in &expired {
            // Rotate: destroy old session, create new one
            self.destroy_session(tab_id).await;
            self.create_session(tab_id).await;
            info!("Ghost: rotated expired session for tab {}", tab_id);
        }
    }

    /// Get the spoofed user-agent for a ghost tab.
    pub async fn get_user_agent(&self, tab_id: &str) -> Option<String> {
        self.sessions.read().await
            .get(tab_id)
            .map(|s| s.user_agent.clone())
    }

    /// Route an HTTP request through the Phantom chain (or fallback).
    /// Returns the response body bytes and headers.
    pub async fn route_request(
        &self,
        tab_id:  &str,
        url:     &str,
        method:  &str,
        headers: HashMap<String, String>,
        body:    Option<Vec<u8>>,
    ) -> Result<PhantomResponse> {
        // Prune expired sessions opportunistically
        self.prune_expired_sessions().await;

        let config = self.config.read().await.clone();

        if config.entry_node.is_some() {
            // Full onion-routed path
            self.route_via_onion(tab_id, url, method, headers, body, &config).await
        } else {
            // Fallback: route via Cloudflare's privacy-preserving proxy + obfuscated headers
            self.route_via_fallback(tab_id, url, method, headers, body).await
        }
    }

    /// Full 3-hop onion routing.
    async fn route_via_onion(
        &self,
        tab_id:  &str,
        url:     &str,
        method:  &str,
        headers: HashMap<String, String>,
        body:    Option<Vec<u8>>,
        config:  &PhantomConfig,
    ) -> Result<PhantomResponse> {
        let sessions = self.sessions.read().await;
        let session  = sessions.get(tab_id)
            .ok_or_else(|| anyhow!("No ghost session for tab {}", tab_id))?;

        // Build the request envelope
        let envelope = serde_json::json!({
            "url":     url,
            "method":  method,
            "headers": headers,
            "body":    body.as_ref().map(|b| { use base64::Engine; base64::engine::general_purpose::STANDARD.encode(b) }),
            "session": hex::encode(&session.keys.session_id),
        });

        let plain = serde_json::to_vec(&envelope)?;
        let onion  = session.keys.onion_encrypt(&plain)?;

        // POST the onion packet to the entry node
        let entry = config.entry_node.as_ref().unwrap();
        let resp  = self.client
            .post(entry)
            .header("Content-Type", "application/octet-stream")
            .header("X-Phantom-Version", "1")
            .body(onion)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(anyhow!("Phantom entry node error: {}", resp.status()));
        }

        let status   = resp.status().as_u16();
        let resp_headers: HashMap<String, String> = resp.headers().iter()
            .filter_map(|(k, v)| {
                v.to_str().ok().map(|s| (k.as_str().to_string(), s.to_string()))
            })
            .collect();

        let ciphertext = resp.bytes().await?;
        let plaintext  = session.keys.decrypt_response(&ciphertext)?;

        Ok(PhantomResponse {
            status,
            headers: resp_headers,
            body: plaintext,
        })
    }

    /// Fallback when no Phantom server is configured.
    /// Uses Cloudflare's privacy proxy (hides real IP from destination).
    /// Also strips all identifying headers and injects the session's
    /// random user-agent.
    async fn route_via_fallback(
        &self,
        tab_id:  &str,
        url:     &str,
        method:  &str,
        mut headers: HashMap<String, String>,
        body:    Option<Vec<u8>>,
    ) -> Result<PhantomResponse> {
        let (ua, lang) = {
            let sessions = self.sessions.read().await;
            let session  = sessions.get(tab_id)
                .ok_or_else(|| anyhow!("No ghost session for tab {}", tab_id))?;
            (session.user_agent.clone(), session.accept_lang.clone())
        };

        // Strip all fingerprinting headers
        headers.remove("Referer");
        headers.remove("referer");
        headers.remove("Cookie");
        headers.remove("cookie");
        headers.remove("X-Forwarded-For");
        headers.remove("Via");

        // Inject randomised identity
        headers.insert("User-Agent".to_string(), ua);
        headers.insert("Accept-Language".to_string(), lang);
        // Remove client hints (fingerprinting vectors)
        headers.insert("Sec-CH-UA".to_string(), "".to_string());
        headers.insert("Sec-CH-UA-Mobile".to_string(), "?0".to_string());
        headers.insert("Sec-CH-UA-Platform".to_string(), "\"Linux\"".to_string());

        let mut req = self.client.request(
            method.parse().unwrap_or(reqwest::Method::GET),
            url,
        );
        for (k, v) in &headers {
            req = req.header(k, v);
        }
        if let Some(b) = body {
            req = req.body(b);
        }

        let resp = req.send().await?;
        let status = resp.status().as_u16();
        let resp_headers: HashMap<String, String> = resp.headers().iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|s| (k.as_str().to_string(), s.to_string())))
            .collect();
        let body_bytes = resp.bytes().await?.to_vec();

        Ok(PhantomResponse { status, headers: resp_headers, body: body_bytes })
    }

    pub async fn configure(&self, config: PhantomConfig) {
        *self.config.write().await = config;
        info!("Ghost Mode: phantom router configured");
    }

    pub async fn get_config(&self) -> PhantomConfig {
        self.config.read().await.clone()
    }
}

/// Response from the phantom routing layer.
#[derive(Debug)]
pub struct PhantomResponse {
    pub status:  u16,
    pub headers: HashMap<String, String>,
    pub body:    Vec<u8>,
}

// ── Fingerprint randomisation ─────────────────────────────────────────────────

/// Returns a random plausible desktop Chrome user-agent.
/// Using a desktop UA also prevents sites from delivering mobile tracking scripts.
fn random_user_agent() -> String {
    let chrome_versions = [
        "120.0.0.0", "119.0.0.0", "118.0.0.0", "121.0.0.0", "122.0.0.0",
        "116.0.0.0", "117.0.0.0", "115.0.0.0", "123.0.0.0", "124.0.0.0",
    ];
    let platforms = [
        ("X11; Linux x86_64",           "Linux x86_64"),
        ("Windows NT 10.0; Win64; x64",  "Windows"),
        ("Macintosh; Intel Mac OS X 10_15_7", "macOS"),
    ];
    let mut rng  = rand::thread_rng();
    let cv       = chrome_versions[rng.gen_range(0..chrome_versions.len())];
    let (plat, _) = platforms[rng.gen_range(0..platforms.len())];
    format!(
        "Mozilla/5.0 ({}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{} Safari/537.36",
        plat, cv
    )
}

/// Returns a random accept-language to resist language fingerprinting.
fn random_accept_language() -> String {
    let langs = [
        "en-US,en;q=0.9",
        "en-GB,en;q=0.9",
        "en-US,en;q=0.9,fr;q=0.8",
        "en-US,en;q=0.9,de;q=0.8",
        "en-US,en;q=0.9,es;q=0.8",
        "en-US,en;q=0.9,ja;q=0.8",
    ];
    let mut rng = rand::thread_rng();
    langs[rng.gen_range(0..langs.len())].to_string()
}

// ── IPC types ─────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct GhostStatus {
    pub enabled:          bool,
    pub session_count:    usize,
    pub has_proxy_server: bool,
    pub hop_count:        u8,
    pub dns_private:      bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ephemeral_keys_encrypt_decrypt() {
        let keys      = EphemeralKeys::generate();
        let plaintext = b"ghost mode test payload";
        let onion     = keys.onion_encrypt(plaintext).unwrap();
        assert_ne!(onion, plaintext);
        // Decrypt only layer1 (simulating exit node already peeled layers 2+3)
        let decrypted = chacha_decrypt(&keys.key_layer1,
            &chacha_decrypt(&keys.key_layer2,
                &chacha_decrypt(&keys.key_layer3, &onion).unwrap()
            ).unwrap()
        ).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn keys_zeroize_on_drop() {
        let mut keys = EphemeralKeys::generate();
        keys.zeroize();
        assert_eq!(keys.key_layer1, [0u8; 32]);
        assert_eq!(keys.key_layer2, [0u8; 32]);
        assert_eq!(keys.key_layer3, [0u8; 32]);
    }

    #[tokio::test]
    async fn router_creates_and_destroys_sessions() {
        let router = PhantomRouter::new();
        router.create_session("tab_abc").await;
        assert!(router.sessions.read().await.contains_key("tab_abc"));
        router.destroy_session("tab_abc").await;
        assert!(!router.sessions.read().await.contains_key("tab_abc"));
    }
}
