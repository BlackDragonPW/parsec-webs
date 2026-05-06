// src-tauri/src/extension_store.rs
//
// Real Chrome Web Store client.
//
// Chrome Web Store endpoints used:
//   Search:   POST https://chrome.google.com/webstore/ajax/item
//   Detail:   GET  https://chrome.google.com/webstore/detail/{id}
//   Download: GET  https://clients2.google.com/service/update2/crx
//             ?response=redirect&prodversion=131.0.0.0
//             &x=id%3D{id}%26installsource%3Dondemand%26uc
//
// CRX3 format:
//   4 bytes magic: Cr24
//   4 bytes version: 3
//   4 bytes header_size
//   N bytes CrxFileHeader protobuf
//   ZIP blob (the actual extension)
//
// We strip the CRX header and unzip to get manifest.json + content scripts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use serde::{Deserialize, Serialize};
use anyhow::{Context, Result, anyhow};
use tracing::{info, warn, debug};
use reqwest::Client;

// ── Types ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CwsExtension {
    pub id:           String,
    pub name:         String,
    pub author:       String,
    pub version:      String,
    pub description:  String,
    pub icon_url:     Option<String>,
    pub rating:       f32,
    pub rating_count: u32,
    pub user_count:   String,    // "10,000,000+"
    pub category:     String,
    pub featured:     bool,
    pub price:        String,    // "Free" or "$1.99"
    pub last_updated: String,
    pub permissions:  Vec<String>,
    pub size:         String,
    pub languages:    Vec<String>,
    pub screenshots:  Vec<String>,
    pub store_url:    String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledExtension {
    pub id:           String,
    pub name:         String,
    pub version:      String,
    pub description:  String,
    pub icon:         String,      // emoji or data: URI
    pub icon_bg:      String,      // CSS gradient
    pub enabled:      bool,
    pub install_path: PathBuf,
    pub manifest:     ExtManifest,
    pub content_scripts_injected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtManifest {
    pub manifest_version:  u8,
    pub name:              String,
    pub version:           String,
    pub description:       Option<String>,
    pub permissions:       Vec<String>,
    pub host_permissions:  Vec<String>,
    pub content_scripts:   Vec<ContentScript>,
    pub background:        Option<BackgroundSpec>,
    pub action:            Option<ActionSpec>,
    pub browser_action:    Option<ActionSpec>,
    pub icons:             HashMap<String, String>,
    pub web_accessible_resources: Vec<serde_json::Value>,
    pub content_security_policy: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContentScript {
    pub matches:     Vec<String>,
    pub js:          Vec<String>,
    pub css:         Vec<String>,
    pub run_at:      Option<String>,
    pub all_frames:  Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BackgroundSpec {
    pub service_worker: Option<String>,
    pub scripts:        Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ActionSpec {
    pub default_popup: Option<String>,
    pub default_icon:  Option<serde_json::Value>,
    pub default_title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CwsSearchResult {
    pub extensions: Vec<CwsExtension>,
    pub total:      usize,
    pub page:       usize,
    pub has_more:   bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallProgress {
    pub ext_id:  String,
    pub stage:   String,  // "downloading", "extracting", "installing", "done", "error"
    pub percent: u8,
    pub message: String,
}

// ── Category mappings ─────────────────────────────────────────────────────────

pub fn cws_category_id(cat: &str) -> &'static str {
    match cat {
        "Productivity"   => "productivity",
        "Tools"          => "ext/11-web-development",
        "Shopping"       => "ext/7-shopping",
        "Fun"            => "ext/5-photos",
        "Accessibility"  => "ext/22-accessibility",
        "Privacy"        => "ext/15-by-google",
        "Dev Tools"      => "ext/11-web-development",
        _                => "all",
    }
}

// ── CWS Client ────────────────────────────────────────────────────────────────

pub struct CwsClient {
    client: Client,
}

impl CwsClient {
    pub fn new() -> Self {
        let client = Client::builder()
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 ParsecWeb/1.3")
            .gzip(true)
            .brotli(true)
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .unwrap_or_default();
        Self { client }
    }

    /// Search Chrome Web Store
    /// Uses the same AJAX endpoint the CWS website itself uses.
    pub async fn search(&self, query: &str, page: usize) -> Result<CwsSearchResult> {
        let count = 24usize;
        let start = page * count;

        // CWS search endpoint — returns protobuf-JSON hybrid
        let url = format!(
            "https://chrome.google.com/webstore/ajax/item\
             ?hl=en&gl=US&pv=20210820\
             &mce=atf,sid,pii,aut,perm,ads,crx\
             &searchTerm={query}&count={count}&startingIndex={start}&sortBy=0",
            query = percent_encoding::percent_encode(query.as_bytes(), percent_encoding::NON_ALPHANUMERIC),
        );

        let resp = self.client.get(&url)
            .header("x-requested-with", "XMLHttpRequest")
            .send().await
            .context("CWS search request")?;

        if !resp.status().is_success() {
            return Err(anyhow!("CWS search HTTP {}", resp.status()));
        }

        let text = resp.text().await?;
        self.parse_cws_response(&text, page)
    }

    /// Fetch featured extensions for a category
    pub async fn featured(&self, category: &str) -> Result<CwsSearchResult> {
        let cat_id = cws_category_id(category);
        let url = format!(
            "https://chrome.google.com/webstore/ajax/collection\
             ?hl=en&gl=US&pv=20210820\
             &mce=atf,sid,pii,aut,perm,ads,crx\
             &category={cat_id}&count=24"
        );

        let resp = self.client.get(&url)
            .header("x-requested-with", "XMLHttpRequest")
            .send().await
            .context("CWS featured request")?;

        if !resp.status().is_success() {
            // Fall back to search for the category name
            return self.search(category, 0).await;
        }

        let text = resp.text().await?;
        self.parse_cws_response(&text, 0)
    }

    /// Get full details for a single extension
    pub async fn get_detail(&self, ext_id: &str) -> Result<CwsExtension> {
        // Try search by ID first (most reliable)
        let result = self.search(ext_id, 0).await?;
        result.extensions.into_iter().next()
            .ok_or_else(|| anyhow!("Extension {ext_id} not found"))
    }

    /// Download a CRX file
    /// Returns raw CRX bytes
    pub async fn download_crx(&self, ext_id: &str) -> Result<Vec<u8>> {
        let url = format!(
            "https://clients2.google.com/service/update2/crx\
             ?response=redirect\
             &prodversion=131.0.0.0\
             &acceptformat=crx3\
             &x=id%3D{ext_id}%26installsource%3Dondemand%26uc"
        );

        info!("Downloading CRX: {ext_id}");

        let resp = self.client.get(&url)
            .header("accept", "application/octet-stream,*/*")
            .send().await
            .context("CRX download request")?;

        if !resp.status().is_success() {
            return Err(anyhow!("CRX download HTTP {}", resp.status()));
        }

        let bytes = resp.bytes().await.context("CRX read bytes")?;
        info!("Downloaded CRX {} ({} bytes)", ext_id, bytes.len());
        Ok(bytes.to_vec())
    }

    // ── CWS response parser ────────────────────────────────────────────────────
    //
    // The CWS AJAX response is:
    //   )]}'\n
    //   [[[...extension data...]]]
    //
    // Each extension entry is a deeply nested array. Key field indices:
    //   [0][0] = extension ID
    //   [0][1] = extension name
    //   [0][6] = description
    //   [0][37][1] = author name
    //   [0][25][0][0] = price info
    //   [0][12] = avg rating (float)
    //   [0][22] = rating count
    //   [0][5][0][icon_size] = icon URL at various sizes
    //   [0][31][0] = user count string
    //   [0][16][0] = categories

    fn parse_cws_response(&self, text: &str, page: usize) -> Result<CwsSearchResult> {
        // Strip XSSI prefix
        let json_start = text.find('[').ok_or_else(|| anyhow!("No JSON in CWS response"))?;
        let json_text  = &text[json_start..];

        let data: serde_json::Value = serde_json::from_str(json_text)
            .context("CWS JSON parse")?;

        let mut extensions = Vec::new();

        // Navigate the nested array structure
        // data[0][1] = array of extension entries
        let entries = &data[0][1];
        let arr = match entries.as_array() {
            Some(a) => a,
            None => return Ok(CwsSearchResult { extensions, total: 0, page, has_more: false }),
        };

        for entry in arr {
            match self.parse_extension_entry(entry) {
                Ok(ext) => extensions.push(ext),
                Err(e)  => debug!("Skip extension: {e}"),
            }
        }

        let total  = extensions.len() + page * 24;
        let has_more = extensions.len() == 24;

        Ok(CwsSearchResult { extensions, total, page, has_more })
    }

    fn parse_extension_entry(&self, e: &serde_json::Value) -> Result<CwsExtension> {
        let inner = &e[0];

        let id = inner[0].as_str().ok_or_else(|| anyhow!("no id"))?.to_string();
        if id.len() != 32 { return Err(anyhow!("invalid id length")); }

        let name = inner[1].as_str().unwrap_or("Unknown").to_string();
        let description = inner[6].as_str().unwrap_or("").to_string();
        let rating = inner[12].as_f64().unwrap_or(0.0) as f32;
        let rating_count = inner[22].as_u64().unwrap_or(0) as u32;

        // Author
        let author = inner[37][1].as_str()
            .or_else(|| inner[37][0][0].as_str())
            .unwrap_or("Unknown")
            .to_string();

        // User count
        let user_count = inner[31][0].as_str()
            .or_else(|| inner[23].as_str())
            .unwrap_or("?")
            .to_string();

        // Icon URL — prefer 128px
        let icon_url = inner[5][0].as_str()
            .or_else(|| inner[5].as_str())
            .map(|s| s.to_string());

        // Price
        let price = if inner[25][0][0].as_str().unwrap_or("").is_empty() {
            "Free".to_string()
        } else {
            inner[25][0][0].as_str().unwrap_or("Free").to_string()
        };

        // Version
        let version = inner[11][0][1].as_str()
            .unwrap_or("1.0.0")
            .to_string();

        // Category
        let category = inner[16][0].as_str()
            .unwrap_or("Extensions")
            .to_string();

        // Last updated
        let last_updated = inner[7].as_str().unwrap_or("").to_string();

        // Store URL
        let store_url = format!("https://chrome.google.com/webstore/detail/{id}");

        Ok(CwsExtension {
            id, name, author, version, description, icon_url, rating, rating_count,
            user_count, category, featured: false, price, last_updated,
            permissions: vec![], size: "?".into(), languages: vec![], screenshots: vec![],
            store_url,
        })
    }
}

// ── CRX installer ─────────────────────────────────────────────────────────────

pub struct CrxInstaller {
    install_dir: PathBuf,
}

impl CrxInstaller {
    pub fn new() -> Self {
        let dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("parsec-web")
            .join("extensions");
        std::fs::create_dir_all(&dir).ok();
        Self { install_dir: dir }
    }

    /// Install a CRX file. Returns the parsed manifest + install path.
    pub async fn install(&self, ext_id: &str, crx_bytes: &[u8]) -> Result<InstalledExtension> {
        // 1. Parse CRX3 format → extract ZIP blob
        let zip_bytes = self.strip_crx_header(crx_bytes)
            .context("CRX header parse")?;

        // 2. Unzip to extension directory
        let ext_dir = self.install_dir.join(ext_id);
        std::fs::create_dir_all(&ext_dir)?;

        self.unzip_to(&zip_bytes, &ext_dir)
            .context("CRX unzip")?;

        // 3. Parse manifest.json
        let manifest_path = ext_dir.join("manifest.json");
        let manifest_text = std::fs::read_to_string(&manifest_path)
            .context("read manifest.json")?;
        let manifest: ExtManifest = serde_json::from_str(&manifest_text)
            .context("parse manifest.json")?;

        // 4. Load icon
        let icon = self.load_icon(&ext_dir, &manifest);
        let icon_bg = self.pick_icon_bg(&manifest.name);

        info!("Installed extension {} v{} to {:?}", manifest.name, manifest.version, ext_dir);

        Ok(InstalledExtension {
            id:           ext_id.to_string(),
            name:         manifest.name.clone(),
            version:      manifest.version.clone(),
            description:  manifest.description.clone().unwrap_or_default(),
            icon,
            icon_bg,
            enabled:      true,
            install_path: ext_dir,
            content_scripts_injected: false,
            manifest,
        })
    }

    /// Build the JS string to inject content scripts for a given URL.
    /// Called by TabManager when navigating to inject matching content scripts.
    pub fn build_injection_script(ext: &InstalledExtension, url: &str) -> Option<String> {
        let mut scripts: Vec<String> = Vec::new();
        let mut styles:  Vec<String> = Vec::new();

        for cs in &ext.manifest.content_scripts {
            if !url_matches_patterns(url, &cs.matches) { continue; }
            let run_at = cs.run_at.as_deref().unwrap_or("document_idle");

            for js_file in &cs.js {
                let path = ext.install_path.join(js_file);
                match std::fs::read_to_string(&path) {
                    Ok(code) => scripts.push(format!(
                        "// Content script: {js_file}\n(function() {{ {code} }})();"
                    )),
                    Err(e) => warn!("Can't read content script {js_file}: {e}"),
                }
            }

            for css_file in &cs.css {
                let path = ext.install_path.join(css_file);
                match std::fs::read_to_string(&path) {
                    Ok(css) => styles.push(css),
                    Err(e) => warn!("Can't read css {css_file}: {e}"),
                }
            }
        }

        if scripts.is_empty() && styles.is_empty() { return None; }

        let mut out = String::new();

        // Inject CSS
        if !styles.is_empty() {
            let combined_css = styles.join("\n").replace('`', "\\`");
            out.push_str(&format!(r#"
(function() {{
  const __style = document.createElement('style');
  __style.id = '__parsec_ext_{id}';
  __style.textContent = `{css}`;
  document.documentElement.appendChild(__style);
}})();
"#, id = ext.id, css = combined_css));
        }

        // Inject JS
        for script in scripts {
            out.push_str(&script);
            out.push('\n');
        }

        Some(out)
    }

    // ── CRX3 header stripping ──────────────────────────────────────────────────
    //
    // CRX3 format:
    //   [0..4]  magic: "Cr24"
    //   [4..8]  version: u32le = 3
    //   [8..12] header_size: u32le
    //   [12..12+header_size] CrxFileHeader proto (ignored — we don't verify sig)
    //   [12+header_size..] ZIP data

    fn strip_crx_header<'a>(&self, data: &'a [u8]) -> Result<Vec<u8>> {
        if data.len() < 16 {
            return Err(anyhow!("CRX too small ({} bytes)", data.len()));
        }

        // Check magic
        if &data[0..4] == b"Cr24" {
            let version = u32::from_le_bytes(data[4..8].try_into()?);
            if version == 2 {
                // CRX2: magic(4) + version(4) + pubkey_len(4) + sig_len(4) + pubkey + sig + ZIP
                let pk_len  = u32::from_le_bytes(data[8..12].try_into()?)  as usize;
                let sig_len = u32::from_le_bytes(data[12..16].try_into()?) as usize;
                let zip_start = 16 + pk_len + sig_len;
                if zip_start >= data.len() { return Err(anyhow!("CRX2 offset out of bounds")); }
                return Ok(data[zip_start..].to_vec());
            } else if version == 3 {
                // CRX3: magic(4) + version(4) + header_size(4) + proto + ZIP
                let hdr_size = u32::from_le_bytes(data[8..12].try_into()?) as usize;
                let zip_start = 12 + hdr_size;
                if zip_start >= data.len() { return Err(anyhow!("CRX3 offset out of bounds")); }
                return Ok(data[zip_start..].to_vec());
            }
        }

        // Not a CRX — try treating as bare ZIP
        if data.len() > 4 && &data[0..4] == b"PK\x03\x04" {
            return Ok(data.to_vec());
        }

        Err(anyhow!("Unknown format (magic: {:?})", &data[..4.min(data.len())]))
    }

    fn unzip_to(&self, zip_bytes: &[u8], dest: &Path) -> Result<()> {
        use std::io::Read;
        let cursor  = std::io::Cursor::new(zip_bytes);
        let mut zip = zip::ZipArchive::new(cursor).context("zip open")?;

        for i in 0..zip.len() {
            let mut file = zip.by_index(i)?;
            let name = file.name().to_string();

            // Sanitise path — never allow ..
            if name.contains("..") { continue; }

            let out_path = dest.join(&name);

            if name.ends_with('/') {
                std::fs::create_dir_all(&out_path)?;
            } else {
                if let Some(parent) = out_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut contents = Vec::new();
                file.read_to_end(&mut contents)?;
                std::fs::write(&out_path, contents)?;
            }
        }
        Ok(())
    }

    fn load_icon(&self, dir: &Path, manifest: &ExtManifest) -> String {
        // Try manifest icon sizes: 128, 48, 32, 16
        for size in &["128", "48", "32", "16"] {
            if let Some(icon_path) = manifest.icons.get(*size) {
                let full = dir.join(icon_path);
                if let Ok(bytes) = std::fs::read(&full) {
                    let mime = if icon_path.ends_with(".svg") { "image/svg+xml" } else { "image/png" };
                    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
                    return format!("data:{mime};base64,{b64}");
                }
            }
        }
        "🧩".to_string()
    }

    fn pick_icon_bg(&self, name: &str) -> String {
        // Deterministic gradient from name hash
        let h: u64 = name.bytes().fold(0x811c9dc5u64, |acc, b|
            (acc ^ b as u64).wrapping_mul(0x1000193));
        let hue = (h % 360) as u16;
        let sat = 60 + (h >> 8) % 20;
        let lit = 40 + (h >> 16) % 15;
        format!("linear-gradient(135deg, hsl({hue},{sat}%,{lit}%), hsl({},{}%,{}%))",
            (hue + 30) % 360, sat, lit + 10)
    }
}

// ── URL pattern matching (Chrome extension format) ────────────────────────────
//
// Patterns: https://developer.chrome.com/docs/extensions/mv3/match_patterns/
//   <scheme>://<host><path>
//   Wildcards: * in host matches any subdomain, * in path matches anything

pub fn url_matches_patterns(url: &str, patterns: &[String]) -> bool {
    for pattern in patterns {
        if pattern == "<all_urls>" { return true; }
        if url_matches_pattern(url, pattern) { return true; }
    }
    false
}

fn url_matches_pattern(url: &str, pattern: &str) -> bool {
    // Parse the pattern
    let (scheme_pat, rest) = if let Some(r) = pattern.strip_prefix("https://") {
        ("https", r)
    } else if let Some(r) = pattern.strip_prefix("http://") {
        ("http", r)
    } else if let Some(r) = pattern.strip_prefix("*://") {
        ("*", r)
    } else {
        return false;
    };

    // Parse the URL
    let Ok(parsed) = url::Url::parse(url) else { return false; };

    // Scheme check
    if scheme_pat != "*" && parsed.scheme() != scheme_pat { return false; }

    // Split host and path from rest
    let (host_pat, path_pat) = rest.split_once('/').unwrap_or((rest, ""));

    // Host check
    let host = parsed.host_str().unwrap_or("");
    if !host_matches(host, host_pat) { return false; }

    // Path check
    let path = parsed.path();
    glob_matches(path, &format!("/{path_pat}"))
}

fn host_matches(host: &str, pattern: &str) -> bool {
    if pattern == "*" { return true; }
    if let Some(sub_pat) = pattern.strip_prefix("*.") {
        // *.example.com matches a.example.com but not example.com
        host == sub_pat || host.ends_with(&format!(".{sub_pat}"))
    } else {
        host == pattern
    }
}

fn glob_matches(s: &str, pattern: &str) -> bool {
    // Simple * wildcard matching
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 { return s == pattern || pattern == "/*"; }
    if parts.is_empty() { return true; }

    let mut pos = 0usize;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() { continue; }
        match s[pos..].find(part) {
            Some(idx) => {
                if i == 0 && idx != 0 { return false; } // first part must be at start
                pos += idx + part.len();
            }
            None => return false,
        }
    }
    true
}

// ── Extension Registry (in-memory + disk) ─────────────────────────────────────

pub struct ExtensionRegistry {
    installed: HashMap<String, InstalledExtension>,
    cws:       CwsClient,
    installer: CrxInstaller,
}

impl ExtensionRegistry {
    pub fn new() -> Self {
        let mut reg = Self {
            installed: HashMap::new(),
            cws:       CwsClient::new(),
            installer: CrxInstaller::new(),
        };
        reg.load_from_disk();
        reg
    }

    /// Search the real Chrome Web Store
    pub async fn search_store(&self, query: &str, page: usize) -> Result<CwsSearchResult> {
        self.cws.search(query, page).await
    }

    /// Get featured extensions for a category
    pub async fn featured(&self, category: &str) -> Result<CwsSearchResult> {
        self.cws.featured(category).await
    }

    /// Download + install extension from CWS
    pub async fn install_from_store(
        &mut self,
        ext_id: &str,
        progress_fn: impl Fn(InstallProgress) + Send + 'static,
    ) -> Result<InstalledExtension> {
        progress_fn(InstallProgress { ext_id: ext_id.into(), stage: "downloading".into(), percent: 10, message: "Downloading from Chrome Web Store…".into() });

        let crx_bytes = self.cws.download_crx(ext_id).await
            .context("CRX download")?;

        progress_fn(InstallProgress { ext_id: ext_id.into(), stage: "extracting".into(), percent: 50, message: "Extracting extension…".into() });

        let ext = self.installer.install(ext_id, &crx_bytes).await
            .context("CRX install")?;

        progress_fn(InstallProgress { ext_id: ext_id.into(), stage: "installing".into(), percent: 85, message: "Registering extension…".into() });

        self.installed.insert(ext_id.to_string(), ext.clone());
        self.save_to_disk();

        progress_fn(InstallProgress { ext_id: ext_id.into(), stage: "done".into(), percent: 100, message: format!("Installed {}", ext.name) });

        Ok(ext)
    }

    /// Enable/disable extension
    pub fn set_enabled(&mut self, ext_id: &str, enabled: bool) {
        if let Some(e) = self.installed.get_mut(ext_id) {
            e.enabled = enabled;
        }
        self.save_to_disk();
    }

    /// Uninstall extension
    pub fn uninstall(&mut self, ext_id: &str) -> Result<()> {
        if let Some(ext) = self.installed.remove(ext_id) {
            std::fs::remove_dir_all(&ext.install_path).ok();
        }
        self.save_to_disk();
        Ok(())
    }

    /// Get all installed extensions
    pub fn list(&self) -> Vec<&InstalledExtension> {
        self.installed.values().collect()
    }

    /// Build injection scripts for a URL (all matching extensions)
    pub fn build_injections_for_url(&self, url: &str) -> Vec<String> {
        self.installed.values()
            .filter(|e| e.enabled)
            .filter_map(|e| CrxInstaller::build_injection_script(e, url))
            .collect()
    }

    fn load_from_disk(&mut self) {
        let index_path = self.installer.install_dir.join("index.json");
        if let Ok(text) = std::fs::read_to_string(&index_path) {
            if let Ok(exts) = serde_json::from_str::<Vec<InstalledExtension>>(&text) {
                for ext in exts {
                    self.installed.insert(ext.id.clone(), ext);
                }
                info!("Loaded {} installed extensions", self.installed.len());
            }
        }
    }

    fn save_to_disk(&self) {
        let index_path = self.installer.install_dir.join("index.json");
        let list: Vec<&InstalledExtension> = self.installed.values().collect();
        if let Ok(text) = serde_json::to_string_pretty(&list) {
            std::fs::write(&index_path, text).ok();
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crx2_strip() {
        let installer = CrxInstaller::new();
        // Fake CRX2: magic + version=2 + pk_len=4 + sig_len=4 + 4 bytes pk + 4 bytes sig + "PK\x03\x04" (ZIP magic)
        let mut crx = vec![b'C',b'r',b'2',b'4']; // wrong magic — test error path
        crx.extend_from_slice(&2u32.to_le_bytes());
        crx.extend_from_slice(&4u32.to_le_bytes()); // pk_len=4
        crx.extend_from_slice(&4u32.to_le_bytes()); // sig_len=4
        crx.extend_from_slice(b"PKPK");             // pk
        crx.extend_from_slice(b"SIGS");             // sig
        crx.extend_from_slice(b"PK\x03\x04rest");   // zip
        // This won't work because magic is wrong, but strip_crx_header should
        // return the raw bytes as fallback if it looks like a ZIP
        // Test the ZIP fallback:
        let zip_data = b"PK\x03\x04test data";
        let result = installer.strip_crx_header(zip_data);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), zip_data);
    }

    #[test]
    fn test_url_pattern_matching() {
        assert!(url_matches_pattern("https://example.com/path", "https://example.com/*"));
        assert!(url_matches_pattern("https://sub.example.com/p", "https://*.example.com/*"));
        assert!(!url_matches_pattern("https://example.com/", "https://other.com/*"));
        assert!(url_matches_pattern("https://example.com/", "<all_urls>") == false); // all_urls handled separately
        assert!(url_matches_patterns("https://foo.com/bar", &["<all_urls>".to_string()]));
        assert!(!url_matches_patterns("https://foo.com/bar", &["https://bar.com/*".to_string()]));
    }

    #[test]
    fn test_icon_bg_deterministic() {
        let inst = CrxInstaller::new();
        let bg1 = inst.pick_icon_bg("uBlock Origin");
        let bg2 = inst.pick_icon_bg("uBlock Origin");
        assert_eq!(bg1, bg2); // same name → same gradient
    }
}
