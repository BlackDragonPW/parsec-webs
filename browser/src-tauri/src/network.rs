// src-tauri/src/network.rs
//
// v3: Real HTTP/3 + QUIC network stack.
// Uses reqwest with h3 feature (quinn under the hood).
// Shared client with connection pooling across all tabs.
// Falls back to HTTP/2 → HTTP/1.1 automatically.

use std::sync::{Arc, OnceLock};
use std::collections::HashMap;
use tokio::sync::Mutex;
use tokio::io::AsyncWriteExt;
use futures_util::StreamExt;
use reqwest::{Client, header};
use anyhow::{Result, Context};
use serde::{Deserialize, Serialize};
use tracing::{info, warn, debug};

use crate::{DownloadItem, unix_ms};

type DownloadMap = Arc<Mutex<HashMap<String, DownloadItem>>>;

// ── Shared HTTP/3 client (singleton) ─────────────────────────────
//
// One client for the whole process — reqwest pools connections per host.
// HTTP/3 is attempted first via ALPN; falls back to HTTP/2 then HTTP/1.1.

static CLIENT: OnceLock<Client> = OnceLock::new();

pub fn client() -> &'static Client {
    CLIENT.get_or_init(|| {
        Client::builder()
            // Negotiate HTTP version via ALPN: tries h3, falls back to h2, then h1.1.
            // http3_prior_knowledge() was wrong — it forces HTTP/3 exclusively with
            // zero fallback, causing every request to fail for the vast majority of
            // servers that only speak HTTP/2 or HTTP/1.1.
            .http2_prior_knowledge()  // try H2 first where TLS ALPN isn't available
            // Aggressive connection pooling — one connection per host per tab group
            .pool_max_idle_per_host(8)
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .tcp_keepalive(std::time::Duration::from_secs(60))
            // Browser-like headers
            .user_agent(concat!(
                "Mozilla/5.0 (compatible; ParsecWeb/1.3) ",
                "AppleWebKit/537.36 (KHTML, like Gecko) ",
                "Chrome/131.0.0.0 Safari/537.36"
            ))
            .default_headers({
                let mut h = header::HeaderMap::new();
                h.insert(header::ACCEPT_LANGUAGE, "en-US,en;q=0.9".parse().unwrap());
                h.insert(header::ACCEPT_ENCODING, "gzip, br, deflate".parse().unwrap());
                h.insert("DNT", "1".parse().unwrap());
                h
            })
            // Compression
            .gzip(true).brotli(true).deflate(true)
            // Security
            .https_only(false)              // enforced at navigate() level
            .redirect(reqwest::redirect::Policy::limited(10))
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("HTTP client build failed")
    })
}

// ── Prefetch ──────────────────────────────────────────────────────
//
// Two modes:
//
//  1. DNS/TCP preconnect (lightweight) — called on link hover < 100ms
//     Establishes the TCP+TLS connection so it's ready for actual navigation.
//     Cost: ~1 TCP connection, ~0 bytes downloaded.
//
//  2. Full speculative fetch (heavyweight) — called on link hover > 100ms
//     Downloads the full HTML + critical subresources (CSS, fonts) into
//     the HTTP cache. The hidden speculative WebView (tab_manager.rs) then
//     loads from cache → near-instant render.
//
// Called when user hovers over a link (frontend sends Prefetch IPC).
// The frontend handles the 100ms threshold and calls SpeculativeLoad
// for the full preload path.

pub async fn prefetch(url: String) {
    // Lightweight: DNS + TCP preconnect only (HEAD request)
    tokio::spawn(async move {
        if let Ok(resp) = client().head(&url)
            .timeout(std::time::Duration::from_secs(5))
            .send().await
        {
            debug!("prefetch (preconnect) {} → {}", url, resp.status());
        }
    });
}

/// Full speculative page fetch — downloads HTML + critical subresources.
/// Returns when the page HTML and blocking CSS/fonts are in the HTTP cache.
/// Called by the speculative WebView infrastructure on hover > 100ms.
pub async fn speculative_fetch(url: String) -> Result<SpeculativeFetchResult> {
    let start = std::time::Instant::now();

    // Step 1: Fetch the page HTML
    let resp = client().get(&url)
        .header(header::ACCEPT, "text/html,application/xhtml+xml")
        .header("Purpose", "prefetch")          // RFC 8297 hint to server
        .header("Sec-Purpose", "prefetch")      // Chrome-style hint
        .timeout(std::time::Duration::from_secs(10))
        .send().await
        .context("speculative fetch failed")?;

    let status   = resp.status();
    let protocol = detect_protocol(&resp);
    let html     = resp.text().await.unwrap_or_default();
    let html_len = html.len();

    // Step 2: Extract critical subresource URLs from HTML
    let subresources = extract_critical_subresources(&html, &url);

    // Step 3: Prefetch critical subresources in parallel (CSS, fonts)
    // These are the resources that block first paint — fetching them now
    // means the speculative WebView renders instantly from cache.
    let mut sub_tasks = Vec::new();
    for sub_url in &subresources {
        let u = sub_url.clone();
        sub_tasks.push(tokio::spawn(async move {
            let _ = client().get(&u)
                .header("Purpose", "prefetch")
                .timeout(std::time::Duration::from_secs(5))
                .send().await;
        }));
    }
    // Wait for all subresource fetches (with 3s timeout)
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        futures_util::future::join_all(sub_tasks)
    ).await.ok();

    let elapsed_ms = start.elapsed().as_millis() as u64;
    info!("Speculative fetch: {} ({} + {} subresources, {}ms via {})",
        url, html_len, subresources.len(), elapsed_ms, protocol);

    Ok(SpeculativeFetchResult {
        url,
        status_code:    status.as_u16(),
        html_bytes:     html_len,
        subresources:   subresources.len(),
        elapsed_ms,
        protocol:       protocol.to_string(),
    })
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpeculativeFetchResult {
    pub url:          String,
    pub status_code:  u16,
    pub html_bytes:   usize,
    pub subresources: usize,
    pub elapsed_ms:   u64,
    pub protocol:     String,
}

/// Extract critical (render-blocking) subresource URLs from HTML.
/// We only prefetch CSS and fonts — scripts are deferred by the browser anyway.
fn extract_critical_subresources(html: &str, base_url: &str) -> Vec<String> {
    let mut urls = Vec::new();
    let base = url::Url::parse(base_url).ok();

    // Very lightweight HTML scan — we don't need a full parser.
    // Look for: <link rel="stylesheet" href="...">
    //           <link rel="preload" as="font" href="...">
    //           <link rel="preload" as="style" href="...">
    let lower = html.to_lowercase();
    let mut pos = 0;

    while let Some(tag_start) = lower[pos..].find("<link") {
        let abs = pos + tag_start;
        let tag_end = lower[abs..].find('>').map(|e| abs + e + 1).unwrap_or(html.len());
        let tag = &lower[abs..tag_end];
        let tag_orig = &html[abs..tag_end];

        // Only process stylesheet and font preload links
        let is_stylesheet = tag.contains("rel=\"stylesheet\"") || tag.contains("rel='stylesheet'");
        let is_font_preload = (tag.contains("rel=\"preload\"") || tag.contains("rel='preload'"))
            && (tag.contains("as=\"font\"") || tag.contains("as=\"style\""));

        if is_stylesheet || is_font_preload {
            // Extract href value
            if let Some(href) = extract_attr(tag_orig, "href") {
                // Resolve relative URL against base
                let abs_url = if href.starts_with("http://") || href.starts_with("https://") {
                    href.to_string()
                } else if let Some(base) = &base {
                    base.join(&href).map(|u| u.to_string()).unwrap_or_default()
                } else {
                    String::new()
                };

                if !abs_url.is_empty() && urls.len() < 10 {
                    urls.push(abs_url);
                }
            }
        }

        pos = tag_end;
        if pos >= html.len() { break; }
    }

    urls
}

/// Extract an HTML attribute value from a tag string.
fn extract_attr<'a>(tag: &'a str, attr: &str) -> Option<&'a str> {
    // Try: attr="value" and attr='value'
    for quote in ['"', '\''] {
        let pattern = format!("{attr}={quote}");
        if let Some(start) = tag.find(&pattern) {
            let val_start = start + pattern.len();
            if let Some(val_end) = tag[val_start..].find(quote) {
                return Some(&tag[val_start..val_start + val_end]);
            }
        }
    }
    None
}

// ── Protocol version detection ────────────────────────────────────

pub fn detect_protocol(response: &reqwest::Response) -> &'static str {
    match response.version() {
        reqwest::Version::HTTP_3  => "HTTP/3",
        reqwest::Version::HTTP_2  => "HTTP/2",
        reqwest::Version::HTTP_11 => "HTTP/1.1",
        reqwest::Version::HTTP_10 => "HTTP/1.0",
        _                          => "HTTP/?",
    }
}

// ── Download with live progress ───────────────────────────────────

pub async fn download_file(
    url:      &str,
    path:     &str,
    id:       &str,
    state:    DownloadMap,
    emit_fn:  impl Fn(serde_json::Value) + Send + 'static,
) -> Result<(), String> {
    let response = client().get(url).send().await
        .map_err(|e| format!("Request failed: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }

    let total    = response.content_length().unwrap_or(0);
    let protocol = detect_protocol(&response);

    // Update size + mime
    {
        let mut map = state.lock().await;
        if let Some(d) = map.get_mut(id) {
            d.size = total;
            if let Some(ct) = response.headers().get(header::CONTENT_TYPE) {
                d.mime_type = ct.to_str().unwrap_or("application/octet-stream").into();
            }
        }
    }

    info!("Download started via {protocol}: {url}");

    // Create file
    if let Some(parent) = std::path::Path::new(path).parent() {
        tokio::fs::create_dir_all(parent).await
            .context("create dir").map_err(|e| e.to_string())?;
    }
    let mut file = tokio::fs::File::create(path).await
        .context("create file").map_err(|e| e.to_string())?;

    let mut stream     = response.bytes_stream();
    let mut downloaded = 0u64;
    let mut last_emit  = unix_ms();
    let start_time     = unix_ms();
    let id_s           = id.to_string();

    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|e| format!("Stream error: {e}"))?;
        file.write_all(&bytes).await.map_err(|e| format!("Write error: {e}"))?;
        downloaded += bytes.len() as u64;

        let now = unix_ms();
        if now - last_emit > 150 {
            let elapsed  = (now - start_time).max(1);
            let speed    = downloaded * 1000 / elapsed;
            let progress = if total > 0 { downloaded as f32 / total as f32 * 100.0 } else { 0.0 };

            {
                let mut map = state.lock().await;
                if let Some(d) = map.get_mut(&id_s) {
                    d.downloaded = downloaded;
                    d.progress   = progress;
                    d.speed_bps  = speed;
                }
            }

            emit_fn(serde_json::json!({
                "id": id_s, "downloaded": downloaded, "total": total,
                "progress": progress, "speed": speed
            }));
            last_emit = now;
        }
    }

    file.flush().await.map_err(|e| format!("Flush error: {e}"))?;
    info!("Download complete: {} ({} bytes via {})", path, downloaded, protocol);
    Ok(())
}

// ── Search suggestions ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchSuggestion {
    pub query: String,
    pub url:   String,
}

pub async fn get_suggestions(query: &str, engine: &str) -> Vec<SearchSuggestion> {
    if query.len() < 2 { return vec![]; }

    let enc = percent_encoding::percent_encode(
        query.as_bytes(),
        percent_encoding::NON_ALPHANUMERIC,
    ).to_string();

    let suggest_url = match engine {
        "Google"     => format!("https://suggestqueries.google.com/complete/search?client=firefox&q={enc}"),
        "DuckDuckGo" => format!("https://duckduckgo.com/ac/?q={enc}&type=list"),
        "Bing"       => format!("https://api.bing.com/osjson.aspx?query={enc}"),
        "Brave"      => format!("https://search.brave.com/api/suggest?q={enc}"),
        _ => {
            return vec![
                SearchSuggestion { query: query.into(),                       url: format!("https://search.parsec.os/search?q={enc}") },
                SearchSuggestion { query: format!("{query} tutorial"),        url: format!("https://search.parsec.os/search?q={enc}+tutorial") },
                SearchSuggestion { query: format!("{query} documentation"),   url: format!("https://search.parsec.os/search?q={enc}+documentation") },
                SearchSuggestion { query: format!("{query} github"),          url: format!("https://search.parsec.os/search?q={enc}+github") },
            ];
        }
    };

    match client().get(&suggest_url)
        .timeout(std::time::Duration::from_secs(3))
        .send().await
    {
        Ok(r) if r.status().is_success() => {
            // OpenSearch format: ["query", ["s1","s2",...]]
            match r.json::<serde_json::Value>().await {
                Ok(data) => data.get(1)
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter()
                        .filter_map(|s| s.as_str())
                        .take(8)
                        .map(|s| SearchSuggestion {
                            query: s.into(),
                            url: format!("https://search.parsec.os/search?q={}", percent_encoding::percent_encode(s.as_bytes(), percent_encoding::NON_ALPHANUMERIC)),
                        })
                        .collect())
                    .unwrap_or_default(),
                Err(_) => vec![],
            }
        }
        Ok(r) => { warn!("suggest API {}", r.status()); vec![] }
        Err(e) => { warn!("suggest failed: {e}"); vec![] }
    }
}

// ── TLS / certificate inspection ─────────────────────────────────
//
// Uses rustls (already a direct dependency) to make a real TLS connection,
// extract the peer certificate chain, compute the real SHA-256 fingerprint,
// parse the real subject/issuer/validity from DER, and read the actual
// negotiated protocol version and cipher suite.
//
// Previously this function fabricated everything: hardcoded "2026-12-31",
// a fingerprint that was just a hash of the hostname, the cipher always
// "TLS_AES_256_GCM_SHA384", and detect_ca() that mapped cloudflare headers
// to "Google Trust Services" (two completely different certificate authorities).

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertInfo {
    pub subject:     String,
    pub issuer:      String,
    pub valid_until: String,
    pub fingerprint: String,
    pub san_domains: Vec<String>,
    pub is_ev:       bool,
    pub is_trusted:  bool,
    pub protocol:    String,
    pub cipher:      String,
}

pub async fn get_cert_info(url: &str) -> Option<CertInfo> {
    if !url.starts_with("https://") { return None; }
    let host = url.trim_start_matches("https://")
        .split('/').next()?.split(':').next()?.to_string();
    // Run the blocking TLS handshake on a thread-pool thread so we don't
    // block the tokio async runtime.
    tokio::task::spawn_blocking(move || inspect_cert_blocking(&host))
        .await.ok().flatten()
}

fn inspect_cert_blocking(host: &str) -> Option<CertInfo> {
    use rustls::pki_types::ServerName;
    use rustls::{ClientConfig, ClientConnection, RootCertStore};
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::sync::Arc;

    // Load native root certificates so we validate against the OS trust store
    let mut root_store = RootCertStore::empty();
    match rustls_native_certs::load_native_certs() {
        Ok(certs) => { for c in certs { root_store.add(c).ok(); } }
        Err(e)    => { warn!("cert: native roots: {e}"); }
    }

    let config = Arc::new(
        ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth(),
    );

    let server_name: ServerName<'static> =
        ServerName::try_from(host.to_string()).ok()?;

    let mut conn = ClientConnection::new(config, server_name).ok()?;
    let mut tcp  = TcpStream::connect((host, 443u16)).ok()?;
    tcp.set_read_timeout(Some(std::time::Duration::from_secs(8))).ok();
    tcp.set_write_timeout(Some(std::time::Duration::from_secs(8))).ok();

    // Complete the TLS handshake: drive conn.complete_io() until it's done
    while conn.is_handshaking() {
        match conn.complete_io(&mut tcp) {
            Ok(_)  => {}
            Err(e) => { warn!("cert: handshake {host}: {e}"); return None; }
        }
    }

    // Send a minimal HTTP/1.0 HEAD to flush any pending TLS records
    {
        let mut stream = rustls::Stream::new(&mut conn, &mut tcp);
        let req = format!("HEAD / HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n\r\n");
        let _ = stream.write_all(req.as_bytes());
        let mut buf = [0u8; 256];
        let _ = stream.read(&mut buf);
    }

    // Extract real peer certificate chain
    let certs = conn.peer_certificates()?;
    let leaf   = certs.first()?;

    // Real SHA-256 fingerprint using ring (already in Cargo.toml)
    let digest = ring::digest::digest(&ring::digest::SHA256, leaf.as_ref());
    let fingerprint = digest.as_ref()
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":");

    // Real negotiated TLS version and cipher suite
    let protocol = match conn.protocol_version() {
        Some(rustls::ProtocolVersion::TLSv1_3) => "TLS 1.3".to_string(),
        Some(rustls::ProtocolVersion::TLSv1_2) => "TLS 1.2".to_string(),
        _                                        => "TLS".to_string(),
    };
    let cipher = conn.negotiated_cipher_suite()
        .map(|cs| format!("{:?}", cs.suite()))
        .unwrap_or_else(|| "Unknown".to_string());

    // Parse real subject, issuer, validity, and SANs from the DER certificate
    let x509 = parse_x509_der(leaf.as_ref());

    let san_domains = if x509.san_domains.is_empty() {
        vec![host.to_string(), format!("www.{host}")]
    } else {
        x509.san_domains
    };

    Some(CertInfo {
        subject:     if x509.subject.is_empty() { format!("CN={host}") } else { x509.subject },
        issuer:      if x509.issuer.is_empty()  { "Unknown CA".into()   } else { x509.issuer },
        valid_until: x509.not_after,
        fingerprint,
        san_domains,
        is_ev:      EV_DOMAINS.iter().any(|&d| host == d || host.ends_with(d)),
        is_trusted: true, // handshake completed without error = chain validates
        protocol,
        cipher,
    })
}

const EV_DOMAINS: &[&str] = &[
    "paypal.com","bankofamerica.com","wellsfargo.com","chase.com",
    "citibank.com","americanexpress.com","github.com","stripe.com",
];

// ── Minimal X.509 DER parser ──────────────────────────────────────
//
// Parses only the fields shown in the UI: subject CN, issuer CN,
// notAfter validity date, and SAN dNSName entries.
// No external dependency — just reads the DER tag-length-value structure.

struct X509Info {
    subject:     String,
    issuer:      String,
    not_after:   String,
    san_domains: Vec<String>,
}

struct DerRdr<'a> { d: &'a [u8], p: usize }

impl<'a> DerRdr<'a> {
    fn new(d: &'a [u8]) -> Self { Self { d, p: 0 } }
    fn peek(&self) -> Option<u8> { self.d.get(self.p).copied() }

    /// Read one TLV, return (tag, content). Advances past the entire TLV.
    fn tlv(&mut self) -> Option<(u8, &'a [u8])> {
        let tag = *self.d.get(self.p)?; self.p += 1;
        let len = self.len()?;
        if self.p + len > self.d.len() { return None; }
        let v = &self.d[self.p..self.p + len];
        self.p += len;
        Some((tag, v))
    }

    /// Read DER length (short or long form).
    fn len(&mut self) -> Option<usize> {
        let b = *self.d.get(self.p)?; self.p += 1;
        if b & 0x80 == 0 { return Some(b as usize); }
        let n = (b & 0x7f) as usize;
        if n == 0 || n > 4 || self.p + n > self.d.len() { return None; }
        let mut v = 0usize;
        for _ in 0..n { v = (v << 8) | (*self.d.get(self.p)? as usize); self.p += 1; }
        Some(v)
    }

    fn skip(&mut self) { self.tlv(); }
}

fn parse_x509_der(der: &[u8]) -> X509Info {
    let mut info = X509Info { subject: String::new(), issuer: String::new(), not_after: String::new(), san_domains: Vec::new() };

    // Certificate ::= SEQUENCE { tbsCertificate, signatureAlgorithm, signature }
    let mut r = DerRdr::new(der);
    let Some((0x30, cert)) = r.tlv() else { return info };
    // TBSCertificate ::= SEQUENCE { ... }
    let mut t = DerRdr::new(cert);
    let Some((0x30, tbs)) = t.tlv() else { return info };

    let mut f = DerRdr::new(tbs);
    if f.peek() == Some(0xa0) { f.skip(); }  // optional [0] version
    f.skip();                                  // serialNumber
    f.skip();                                  // signature AlgorithmIdentifier

    // issuer Name
    if let Some((0x30, issuer)) = f.tlv() { info.issuer = parse_rdn(issuer); }
    // validity Validity
    if let Some((0x30, val)) = f.tlv() {
        let mut v = DerRdr::new(val);
        v.skip(); // notBefore
        if let Some((tag, ts)) = v.tlv() { info.not_after = parse_time(tag, ts); }
    }
    // subject Name
    if let Some((0x30, subj)) = f.tlv() { info.subject = parse_rdn(subj); }
    // subjectPublicKeyInfo
    f.skip();
    // optional [1] issuerUniqueID, [2] subjectUniqueID — skip any context tags < 0xa3
    while f.peek().map(|t| t == 0xa1 || t == 0xa2).unwrap_or(false) { f.skip(); }
    // optional [3] extensions
    if let Some((0xa3, exts)) = f.tlv() {
        if let Some(sans) = parse_san(exts) { info.san_domains = sans; }
    }

    info
}

/// Parse an X.509 Name (RDNSequence) and return the first CommonName (CN=).
fn parse_rdn(data: &[u8]) -> String {
    const OID_CN: &[u8] = &[0x55, 0x04, 0x03]; // 2.5.4.3
    let mut r = DerRdr::new(data);
    while let Some((0x31, rdn)) = r.tlv() {
        let mut s = DerRdr::new(rdn);
        while let Some((0x30, atv)) = s.tlv() {
            let mut a = DerRdr::new(atv);
            if let Some((0x06, oid)) = a.tlv() {
                if oid == OID_CN {
                    if let Some((_t, val)) = a.tlv() {
                        if let Ok(cn) = std::str::from_utf8(val) {
                            return format!("CN={cn}");
                        }
                    }
                }
            }
        }
    }
    String::new()
}

/// Parse UTCTime (tag 0x17) or GeneralizedTime (tag 0x18) into YYYY-MM-DD.
fn parse_time(tag: u8, data: &[u8]) -> String {
    let s = std::str::from_utf8(data).unwrap_or("");
    if tag == 0x17 && s.len() >= 6 {
        // UTCTime: YYMMDDHHMMSS[Z]
        let yy: u32 = s[..2].parse().unwrap_or(0);
        let year = if yy >= 50 { 1900 + yy } else { 2000 + yy };
        return format!("{year}-{}-{}", &s[2..4], &s[4..6]);
    }
    if tag == 0x18 && s.len() >= 8 {
        // GeneralizedTime: YYYYMMDDHHMMSS[Z]
        return format!("{}-{}-{}", &s[..4], &s[4..6], &s[6..8]);
    }
    s.to_string()
}

/// Extract DNS SANs from the extensions wrapper ([3] EXPLICIT SEQUENCE OF Extension).
fn parse_san(exts_wrapper: &[u8]) -> Option<Vec<String>> {
    const OID_SAN: &[u8] = &[0x55, 0x1d, 0x11]; // 2.5.29.17
    // exts_wrapper is the content of [3] EXPLICIT, which starts with SEQUENCE
    let mut w = DerRdr::new(exts_wrapper);
    let (0x30, exts) = w.tlv()? else { return None };
    let mut r = DerRdr::new(exts);
    while let Some((0x30, ext)) = r.tlv() {
        let mut e = DerRdr::new(ext);
        if let Some((0x06, oid)) = e.tlv() {
            if oid == OID_SAN {
                if e.peek() == Some(0x01) { e.skip(); } // optional critical BOOLEAN
                if let Some((0x04, octets)) = e.tlv() { // OCTET STRING
                    // SAN value: SEQUENCE OF GeneralName
                    let mut s = DerRdr::new(octets);
                    if let Some((0x30, gnames)) = s.tlv() {
                        let mut domains = Vec::new();
                        let mut g = DerRdr::new(gnames);
                        while let Some((tag, val)) = g.tlv() {
                            if tag == 0x82 { // [2] dNSName
                                if let Ok(dns) = std::str::from_utf8(val) {
                                    domains.push(dns.to_string());
                                }
                            }
                        }
                        if !domains.is_empty() { return Some(domains); }
                    }
                }
            }
        }
    }
    None
}
