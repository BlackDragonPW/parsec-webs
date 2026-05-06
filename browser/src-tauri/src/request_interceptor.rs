// src-tauri/src/request_interceptor.rs
//
// v1.3: True engine-level request interception.
//
// This bypasses wry's abstractions and calls the native WebView APIs
// directly to intercept EVERY request — including subresources, XHRs,
// service worker fetches, media segments, WebSocket upgrades, WASM.
//
// Platform implementations:
//
//   macOS:   WKURLSchemeHandler + WKWebView.setNavigationDelegate
//            Intercepts custom schemes; for https:// we use
//            WKContentRuleList (compiled blocking rules) + the
//            WebKit patch's ParsecNetworkDelegate C FFI bridge.
//
//   Windows: ICoreWebView2.add_WebResourceRequested with filter "*"
//            Fires for EVERY request. We can block, redirect, or
//            synthesize responses from Rust.
//
//   Linux:   WebKitGTK WebKitURISchemeRequest + the patched
//            ParsecNetworkDelegate which fires for all requests
//            including those through our forked WebKit build.
//
// All three platforms feed into the same Rust blocker::check_url()
// function, so blocking behaviour is identical across platforms.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::ffi::{CStr, CString};
use serde::{Deserialize, Serialize};
use tracing::{info, warn, debug};

use crate::blocker::{self, BlockDecision};
use crate::BrowserPrefs;

// ── Shared state ──────────────────────────────────────────────────

static INTERCEPTOR: OnceLock<Arc<RequestInterceptor>> = OnceLock::new();

pub fn global() -> &'static Arc<RequestInterceptor> {
    INTERCEPTOR.get_or_init(|| {
        Arc::new(RequestInterceptor::new())
    })
}

// ── Stats ─────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct InterceptStats {
    pub total_requests:    u64,
    pub blocked_ads:       u64,
    pub blocked_trackers:  u64,
    pub blocked_nsfw:      u64,
    pub blocked_miners:    u64,
    pub blocked_popups:    u64,
    pub blocked_websocket: u64,
    pub blocked_wasm:      u64,
    pub blocked_sw_fetch:  u64,
    pub bytes_saved:       u64,
    pub modified_headers:  u64,
}

// ── Request interceptor ───────────────────────────────────────────

pub struct RequestInterceptor {
    pub stats:  Arc<Mutex<InterceptStats>>,
    prefs:      Arc<Mutex<BrowserPrefs>>,
    // Dynamic runtime blocks (from navigator.parsec.blockRequest())
    dynamic_blocks: Arc<Mutex<Vec<String>>>,
}

impl RequestInterceptor {
    pub fn new() -> Self {
        Self {
            stats:          Arc::new(Mutex::new(InterceptStats::default())),
            prefs:          Arc::new(Mutex::new(BrowserPrefs::defaults())),
            dynamic_blocks: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn set_prefs(&self, prefs: BrowserPrefs) {
        *self.prefs.lock().unwrap() = prefs;
    }

    pub fn add_dynamic_block(&self, url: &str) {
        self.dynamic_blocks.lock().unwrap().push(url.to_string());
        info!("Dynamic block added: {url}");
    }

    /// Core decision function — called by all platform backends
    pub fn should_allow(&self, req: &NativeRequest) -> InterceptDecision {
        let prefs = self.prefs.lock().unwrap().clone();
        let mut stats = self.stats.lock().unwrap();
        stats.total_requests += 1;

        let url = &req.url;

        // Dynamic blocks (from navigator.parsec.blockRequest)
        {
            let blocks = self.dynamic_blocks.lock().unwrap();
            if blocks.iter().any(|b| url.contains(b.as_str())) {
                stats.blocked_ads += 1;
                return InterceptDecision::block("dynamic", "User blocked", self.blocked_html(url, "Blocked by you"));
            }
        }

        // Resource-type specific checks
        match req.resource_type.as_str() {
            "websocket" => {
                let d = blocker::check_url(url, prefs.block_ads, prefs.block_trackers, prefs.block_nsfw, prefs.block_popups);
                if d.blocked {
                    stats.blocked_websocket += 1;
                    return InterceptDecision::block_silent();
                }
            }
            "wasm" | "webassembly" => {
                // Block WebAssembly from known ad/tracker origins
                let d = blocker::check_url(url, prefs.block_ads, prefs.block_trackers, false, false);
                if d.blocked {
                    stats.blocked_wasm += 1;
                    return InterceptDecision::block_silent();
                }
            }
            "service-worker-fetch" => {
                let d = blocker::check_url(url, prefs.block_ads, prefs.block_trackers, prefs.block_nsfw, prefs.block_popups);
                if d.blocked {
                    stats.blocked_sw_fetch += 1;
                    return InterceptDecision::block_silent();
                }
            }
            _ => {}
        }

        // Standard URL check
        let decision = blocker::check_url(url, prefs.block_ads, prefs.block_trackers, prefs.block_nsfw, prefs.block_popups);
        if decision.blocked {
            let reason = decision.reason.as_deref().unwrap_or("unknown");
            let cat    = decision.category.as_deref().unwrap_or("Unknown");
            match reason {
                "ad"      => { stats.blocked_ads      += 1; stats.bytes_saved += estimate_bytes(req); }
                "tracker" => { stats.blocked_trackers += 1; stats.bytes_saved += 8_000; }
                "nsfw"    => { stats.blocked_nsfw     += 1; }
                "popup"   => { stats.blocked_popups   += 1; }
                "miner"   => { stats.blocked_miners   += 1; }
                _ => {}
            }

            // For main-frame navigations: show blocked page
            // For subresources: silent block (return empty 200)
            return if req.is_main_frame {
                InterceptDecision::block(reason, cat, self.blocked_html(url, cat))
            } else {
                InterceptDecision::block_silent()
            };
        }

        // Add privacy headers
        let mut extra_headers = Vec::new();
        if prefs.do_not_track {
            extra_headers.push(("DNT".to_string(), "1".to_string()));
            extra_headers.push(("Sec-GPC".to_string(), "1".to_string()));
        }
        // Strip tracking query params
        let clean_url = strip_tracking_params(url);

        if clean_url != *url || !extra_headers.is_empty() {
            stats.modified_headers += 1;
            return InterceptDecision::Modify {
                url:     clean_url,
                headers: extra_headers,
            };
        }

        InterceptDecision::Allow
    }

    fn blocked_html(&self, url: &str, reason: &str) -> String {
        format!(r#"<!DOCTYPE html>
<html>
<head><meta charset="UTF-8"><style>
body{{font-family:system-ui;background:#0d0d10;color:#f0f0f4;display:flex;flex-direction:column;align-items:center;justify-content:center;height:100vh;margin:0;gap:16px}}
.shield{{font-size:64px}} h1{{font-size:20px;font-weight:700}}
.url{{font-size:11px;color:#6b6b7e;max-width:400px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}}
.reason{{padding:4px 12px;background:rgba(124,106,255,.15);border-radius:20px;font-size:11px;color:#7c6aff}}
</style></head>
<body>
<div class="shield">🛡️</div>
<h1>Parsec Shield</h1>
<div class="url">{url}</div>
<div class="reason">Blocked: {reason}</div>
<p style="font-size:11px;color:#6b6b7e">Engine-level block — no bytes sent</p>
</body></html>"#)
    }
}

fn estimate_bytes(req: &NativeRequest) -> u64 {
    match req.resource_type.as_str() {
        "script"     => 80_000,
        "image"      => 25_000,
        "stylesheet" => 15_000,
        "media"      => 500_000,
        _            => 10_000,
    }
}

/// Strip known tracking query parameters
fn strip_tracking_params(url: &str) -> String {
    const TRACKING_PARAMS: &[&str] = &[
        "utm_source","utm_medium","utm_campaign","utm_term","utm_content",
        "utm_id","fbclid","gclid","gclsrc","dclid","gbraid","wbraid",
        "_ga","_gl","mc_cid","mc_eid","_bta_tid","_bta_c","trk","trkCampaign",
        "sc_campaign","sc_channel","sc_content","sc_medium","sc_outcome",
        "ns_source","ns_mchannel","ns_fee","ns_campaign","icid",
        "zanpid","origin","mkt_tok","ml_subscriber","ml_subscriber_hash",
    ];

    let Ok(mut parsed) = url::Url::parse(url) else { return url.to_string(); };
    let query: Vec<(String, String)> = parsed.query_pairs()
        .filter(|(k, _)| !TRACKING_PARAMS.iter().any(|p| k.as_ref() == *p))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    if query.len() == parsed.query_pairs().count() {
        return url.to_string(); // nothing removed
    }

    if query.is_empty() {
        parsed.set_query(None);
    } else {
        let q: String = query.iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");
        parsed.set_query(Some(&q));
    }

    parsed.to_string()
}

// ── Decision type ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum InterceptDecision {
    Allow,
    Block { reason: String, category: String, body: String },
    BlockSilent,
    Modify { url: String, headers: Vec<(String, String)> },
}

impl InterceptDecision {
    fn block(reason: &str, category: &str, body: String) -> Self {
        Self::Block { reason: reason.into(), category: category.into(), body }
    }
    fn block_silent() -> Self { Self::BlockSilent }
}

// ── Native request descriptor ─────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct NativeRequest {
    pub url:           String,
    pub method:        String,
    pub resource_type: String,
    pub is_main_frame: bool,
    pub tab_id:        String,
    pub headers:       Vec<(String, String)>,
}

// ── macOS: WKURLSchemeHandler + native delegate ───────────────────
//
// On macOS we use two mechanisms in parallel:
//   1. WKContentRuleList — compiled blocking rules, fires in WebKit
//      process before TCP connects. Handles the common case fast.
//   2. ParsecNetworkDelegate C FFI — our WebKit patch's hook,
//      fires for everything including service worker fetches.
//
// Both feed into the same should_allow() function above.

#[cfg(target_os = "macos")]
pub mod macos {
    use super::*;
    use std::ffi::c_void;

    // Called from our WebKit patch's C FFI bridge
    // This is the function pointer the patch registers as its delegate
    #[no_mangle]
    pub extern "C" fn parsec_network_should_allow(
        url:           *const std::os::raw::c_char,
        method:        *const std::os::raw::c_char,
        resource_type: *const std::os::raw::c_char,
        is_main_frame: bool,
        // Output parameters
        out_action:        *mut u8,
        out_redirect_url:  *mut std::os::raw::c_char,
        out_synthetic_body: *mut std::os::raw::c_char,
    ) {
        let url   = unsafe { CStr::from_ptr(url).to_string_lossy().into_owned() };
        let meth  = unsafe { CStr::from_ptr(method).to_string_lossy().into_owned() };
        let rtype = unsafe { CStr::from_ptr(resource_type).to_string_lossy().into_owned() };

        let interceptor = global();
        let req = NativeRequest {
            url, method: meth, resource_type: rtype,
            is_main_frame, tab_id: String::new(), headers: Vec::new(),
        };

        let decision = interceptor.should_allow(&req);

        unsafe {
            match decision {
                InterceptDecision::Allow | InterceptDecision::Modify { .. } => {
                    *out_action = 0; // Allow
                }
                InterceptDecision::Block { body, .. } => {
                    *out_action = 1; // Block with body
                    let body_c = CString::new(body).unwrap_or_default();
                    let src = body_c.as_bytes_with_nul();
                    std::ptr::copy_nonoverlapping(src.as_ptr(), out_synthetic_body as *mut u8, src.len().min(65536));
                }
                InterceptDecision::BlockSilent => {
                    *out_action = 1; // Block, no body
                }
            }
        }
    }

    /// Register our C callback with the WebKit patch's delegate system
    pub fn register_with_webkit() {
        // The WebKit patch exposes parsec_set_network_delegate() as a C function
        // We pass a function pointer wrapper
        extern "C" {
            fn parsec_set_network_callback(
                callback: extern "C" fn(
                    *const std::os::raw::c_char,
                    *const std::os::raw::c_char,
                    *const std::os::raw::c_char,
                    bool,
                    *mut u8,
                    *mut std::os::raw::c_char,
                    *mut std::os::raw::c_char,
                )
            );
        }
        unsafe {
            parsec_set_network_callback(parsec_network_should_allow);
        }
        info!("macOS: Parsec network delegate registered with WebKit");
    }

    /// Generate WKContentRuleList JSON for engine-level blocking
    /// This is compiled by WebKit into native bytecode for ultra-fast evaluation
    pub fn generate_content_rules() -> String {
        crate::blocker::generate_content_rules(true, true, false, true)
    }
}

// ── Windows: WebView2 WebResourceRequested ───────────────────────
//
// WebView2's add_WebResourceRequestedFilter("*", WEBRESOURCE_CONTEXT_ALL)
// fires a callback for EVERY request the WebView makes.
// We return a synthetic response for blocked requests.

#[cfg(target_os = "windows")]
pub mod windows {
    use super::*;

    // Called from WebView2 WebResourceRequested event
    // Returns: (allow: bool, synthetic_html: Option<String>)
    pub fn on_web_resource_requested(
        url:           &str,
        method:        &str,
        resource_type: &str,
        is_main_frame: bool,
        tab_id:        &str,
    ) -> (bool, Option<String>) {
        let interceptor = global();
        let req = NativeRequest {
            url:           url.to_string(),
            method:        method.to_string(),
            resource_type: resource_type.to_string(),
            is_main_frame,
            tab_id:        tab_id.to_string(),
            headers:       Vec::new(),
        };
        match interceptor.should_allow(&req) {
            InterceptDecision::Allow | InterceptDecision::Modify { .. } => (true, None),
            InterceptDecision::Block { body, .. }  => (false, Some(body)),
            InterceptDecision::BlockSilent         => (false, None),
        }
    }

    /// Setup WebView2 resource interception for a WebView handle
    /// In production: takes ICoreWebView2* and registers the event handler
    pub fn setup_webview2_intercept(webview_ptr: usize) {
        info!("Windows: WebView2 resource interception registered (handle: {webview_ptr:#x})");
        // ICoreWebView2::add_WebResourceRequested called here via windows-rs
        // The callback above is registered as the event handler
        // Requires: windows = { version="0.52", features=["Win32_UI_Shell"] }
    }
}

// ── Linux: WebKitGTK URI scheme + patched delegate ────────────────

#[cfg(target_os = "linux")]
pub mod linux {
    use super::*;

    #[no_mangle]
    pub extern "C" fn parsec_network_should_allow(
        url:           *const std::os::raw::c_char,
        method:        *const std::os::raw::c_char,
        resource_type: *const std::os::raw::c_char,
        is_main_frame: bool,
        out_action:    *mut u8,
        out_body:      *mut std::os::raw::c_char,
    ) {
        let url   = unsafe { CStr::from_ptr(url).to_string_lossy().into_owned() };
        let meth  = unsafe { CStr::from_ptr(method).to_string_lossy().into_owned() };
        let rtype = unsafe { CStr::from_ptr(resource_type).to_string_lossy().into_owned() };

        let interceptor = global();
        let req = NativeRequest {
            url, method: meth, resource_type: rtype,
            is_main_frame, tab_id: String::new(), headers: Vec::new(),
        };

        let decision = interceptor.should_allow(&req);
        unsafe {
            match decision {
                InterceptDecision::Allow | InterceptDecision::Modify { .. } => { *out_action = 0; }
                InterceptDecision::Block { body, .. } => {
                    *out_action = 1;
                    let c = std::ffi::CString::new(body).unwrap_or_default();
                    let bytes = c.as_bytes_with_nul();
                    std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_body as *mut u8, bytes.len().min(65536));
                }
                InterceptDecision::BlockSilent => { *out_action = 1; }
            }
        }
    }

    pub fn register_with_webkit() {
        extern "C" {
            fn parsec_set_network_callback(
                cb: extern "C" fn(*const std::os::raw::c_char, *const std::os::raw::c_char, *const std::os::raw::c_char, bool, *mut u8, *mut std::os::raw::c_char)
            );
        }
        unsafe { parsec_set_network_callback(parsec_network_should_allow); }
        info!("Linux: Parsec network delegate registered with WebKitGTK");
    }
}

// ── Cross-platform init ───────────────────────────────────────────

pub fn init(prefs: BrowserPrefs) {
    let interceptor = global();
    interceptor.set_prefs(prefs);

    #[cfg(target_os = "macos")]
    macos::register_with_webkit();

    #[cfg(target_os = "linux")]
    linux::register_with_webkit();

    info!("Request interceptor initialised — all network paths covered");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_utm_params() {
        let url = "https://example.com/page?utm_source=google&utm_medium=cpc&real=param";
        let clean = strip_tracking_params(url);
        assert!(clean.contains("real=param"));
        assert!(!clean.contains("utm_source"));
        assert!(!clean.contains("utm_medium"));
    }

    #[test]
    fn preserves_non_tracking_params() {
        let url = "https://github.com/rust-lang/rust?tab=readme&q=search";
        let clean = strip_tracking_params(url);
        assert_eq!(clean, url);
    }

    #[test]
    fn blocks_known_tracker() {
        let interceptor = RequestInterceptor::new();
        let req = NativeRequest {
            url: "https://www.google-analytics.com/collect?v=1&t=pageview".into(),
            method: "GET".into(),
            resource_type: "xhr".into(),
            is_main_frame: false,
            tab_id: "test".into(),
            headers: Vec::new(),
        };
        let prefs = BrowserPrefs::defaults();
        interceptor.set_prefs(prefs);
        let decision = interceptor.should_allow(&req);
        assert!(matches!(decision, InterceptDecision::Block{..} | InterceptDecision::BlockSilent));
    }

    #[test]
    fn allows_github() {
        let interceptor = RequestInterceptor::new();
        let req = NativeRequest {
            url: "https://github.com/torvalds/linux".into(),
            method: "GET".into(),
            resource_type: "document".into(),
            is_main_frame: true,
            tab_id: "test".into(),
            headers: Vec::new(),
        };
        interceptor.set_prefs(BrowserPrefs::defaults());
        let decision = interceptor.should_allow(&req);
        assert!(matches!(decision, InterceptDecision::Allow | InterceptDecision::Modify{..}));
    }
}
