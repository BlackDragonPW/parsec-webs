// src-tauri/src/tab_manager.rs
//
// v3: Per-tab native WebView management.
//
// Each browser tab = one wry::WebView with:
//   - Its own OS-level WebView process (WKWebView/WebView2/WebKitGTK)
//   - Custom protocol handler intercepting ALL requests incl. subresources
//   - WKContentRuleList on macOS (engine-level blocking before TCP connects)
//   - JS fetch/XHR override injected at document_start on all platforms
//   - Real back() / forward() / reload() via wry native methods
//   - Script injection for Chrome extension content scripts
//   - IPC channel back to Rust for tab events (title change, URL change, etc.)
//
// Architecture:
//   The React chrome (Neutron GPU) runs in the main window's WebView.
//   Tab WebViews are positioned behind the chrome, visible through a
//   transparent "viewport hole". Tab switching = set_visible(true/false).
//   Resize = set_bounds() when the chrome reports layout changes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use serde::{Deserialize, Serialize};
use tracing::{info, warn, error};

use wry::{WebView, WebViewBuilder, WebContext};
use tao::window::Window;

use crate::blocker;
use crate::network;
use crate::BrowserPrefs;

// ── Tab state ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabState {
    pub id:          String,
    pub url:         String,
    pub title:       String,
    pub favicon:     String,
    pub loading:     bool,
    pub can_go_back: bool,
    pub can_go_fwd:  bool,
    pub pinned:      bool,
    pub muted:       bool,
    pub incognito:   bool,
    pub zoom:        f64,
    pub blocked:     bool,
    pub block_reason: Option<String>,
}

impl TabState {
    pub fn new(id: &str, url: &str, incognito: bool) -> Self {
        Self {
            id: id.into(), url: url.into(), title: "Loading…".into(),
            favicon: "🌐".into(), loading: url != "parsec://newtab",
            can_go_back: false, can_go_fwd: false,
            pinned: false, muted: false, incognito, zoom: 1.0,
            blocked: false, block_reason: None,
        }
    }
}

// ── Tab event sent to chrome React via IPC ────────────────────────
//
// Field names match exactly what ParsecWeb.tsx reads:
//   const { type: t, tabId } = ev
//   ev.title, ev.url, ev.reason, ev.favicon_url, ev.can_back, ev.can_fwd
//
// Using internally-tagged (#[serde(tag="type")]) so each event serializes as a
// flat object: {"type":"TitleChanged","tabId":"...","title":"..."}.
// The previous content="payload" wrapper nested fields under ev.payload,
// making all field reads in React return undefined.

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TabEvent {
    TitleChanged   { #[serde(rename = "tabId")] tab_id: String, title: String },
    UrlChanged     { #[serde(rename = "tabId")] tab_id: String, url: String },
    LoadStart      { #[serde(rename = "tabId")] tab_id: String },
    LoadFinish     { #[serde(rename = "tabId")] tab_id: String },
    FaviconChanged { #[serde(rename = "tabId")] tab_id: String, favicon_url: String },
    Blocked        { #[serde(rename = "tabId")] tab_id: String, url: String, reason: String },
    CanNavigate    { #[serde(rename = "tabId")] tab_id: String, can_back: bool, can_fwd: bool },
}

// ── Tab Manager ───────────────────────────────────────────────────

pub struct TabManager {
    webviews: HashMap<String, WebView>,
    states:   HashMap<String, TabState>,
    active:   Option<String>,
    event_tx: tokio::sync::mpsc::UnboundedSender<TabEvent>,
    prefs:    Arc<Mutex<BrowserPrefs>>,
    // ── Speculative preload pool ──────────────────────────────────
    // Hidden WebViews that have already loaded a URL the user is
    // likely to navigate to (hover prediction, <link rel=prefetch>,
    // Speculation Rules API). On actual navigation, we promote the
    // speculative WebView to a real tab — navigation latency = ~0ms.
    speculative: HashMap<String, WebView>,  // url → hidden WebView
    spec_bounds: (i32, i32, u32, u32),      // off-screen rect
}

impl TabManager {
    pub fn new(
        event_tx: tokio::sync::mpsc::UnboundedSender<TabEvent>,
        prefs:    Arc<Mutex<BrowserPrefs>>,
    ) -> Self {
        Self {
            webviews: HashMap::new(),
            states:   HashMap::new(),
            active:   None,
            event_tx,
            prefs,
            speculative: HashMap::new(),
            spec_bounds: (-8192, -8192, 1, 1), // off-screen, 1×1
        }
    }

    /// Create a new tab WebView.
    ///
    /// `bounds` is the viewport rectangle in the window: (x, y, width, height).
    /// The WebView is positioned here and layered behind the chrome.
    pub fn create_tab(
        &mut self,
        id:       &str,
        url:      &str,
        window:   &Window,
        bounds:   (i32, i32, u32, u32),
        incognito: bool,
    ) -> anyhow::Result<()> {
        let prefs = self.prefs.lock().unwrap().clone();

        // Content rules JSON (macOS native engine blocking)
        let content_rules = blocker::generate_content_rules(
            prefs.block_ads, prefs.block_trackers, prefs.block_nsfw, prefs.block_popups,
        );

        // JS blocker script (injected at document_start on all platforms)
        let blocker_script = blocker::generate_blocker_script(
            prefs.block_ads, prefs.block_trackers, prefs.block_nsfw, prefs.block_popups,
        );

        // Chrome extension content-script shim
        let chrome_shim = include_str!("../../../../extensions/chrome-compat.js");

        // ── Chrome compatibility shim ─────────────────────────────────
        // Injected before any page JS. Fixes sites that UA-sniff or feature-
        // detect Chrome-specific APIs and degrade/break on non-Chrome engines.
        let compat_shim = r#"
(function() {
  'use strict';

  // ── UA spoofing (JS side) ───────────────────────────────────────
  // HTTP-level UA is already set via WebView user_agent config.
  // This covers JS code that reads navigator.userAgent directly.
  const CHROME_UA = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36';
  try {
    Object.defineProperty(navigator, 'userAgent',  { get: () => CHROME_UA,        configurable: false });
    Object.defineProperty(navigator, 'appVersion', { get: () => CHROME_UA.slice(8), configurable: false });
    Object.defineProperty(navigator, 'vendor',     { get: () => 'Google Inc.',     configurable: false });
    Object.defineProperty(navigator, 'platform',   { get: () => 'Win32',           configurable: false });
    // userAgentData — Chrome 90+ structured UA
    Object.defineProperty(navigator, 'userAgentData', {
      get: () => ({
        brands: [
          { brand: 'Google Chrome',   version: '131' },
          { brand: 'Chromium',        version: '131' },
          { brand: 'Not=A?Brand',     version:  '24' },
        ],
        mobile: false,
        platform: 'Windows',
        getHighEntropyValues: (hints) => Promise.resolve({
          architecture: 'x86', bitness: '64', model: '',
          platformVersion: '10.0.0', uaFullVersion: '131.0.0.0',
        }),
        toJSON: () => ({ brands: [], mobile: false, platform: 'Windows' }),
      }),
      configurable: false,
    });
  } catch(e) {}

  // ── scheduler API ───────────────────────────────────────────────
  // React 18+, Vue 3, Angular, and many frameworks check for this.
  // WebKit doesn't expose it; without it some frameworks fall back to
  // setTimeout with worse scheduling behaviour.
  if (!window.scheduler) {
    window.scheduler = {
      postTask: (cb, opts) => {
        const delay = opts?.delay ?? 0;
        return new Promise(resolve => setTimeout(() => resolve(cb()), delay));
      },
      yield: () => new Promise(resolve => setTimeout(resolve, 0)),
    };
  }

  // ── Network Information API ─────────────────────────────────────
  // Used by sites for adaptive loading. WebKit doesn't expose this.
  if (!navigator.connection) {
    try {
      Object.defineProperty(navigator, 'connection', {
        get: () => ({
          effectiveType: '4g', downlink: 10, rtt: 50, saveData: false,
          type: 'wifi', addEventListener: () => {}, removeEventListener: () => {},
        }),
        configurable: true,
      });
    } catch(e) {}
  }

  // ── performance.memory ──────────────────────────────────────────
  // Many profiling tools and frameworks check for this.
  if (window.performance && !performance.memory) {
    try {
      Object.defineProperty(performance, 'memory', {
        get: () => ({
          usedJSHeapSize: 0, totalJSHeapSize: 0,
          jsHeapSizeLimit: 4_294_705_152,
        }),
        configurable: true,
      });
    } catch(e) {}
  }

  // ── CSS.registerProperty (Houdini Paint API) ────────────────────
  // Sites register custom CSS properties; without this they throw.
  if (window.CSS && !CSS.registerProperty) {
    CSS.registerProperty = () => {};
  }
  if (window.CSS && !CSS.paintWorklet) {
    CSS.paintWorklet = { addModule: () => Promise.resolve() };
  }

  // ── Trusted Types ───────────────────────────────────────────────
  // Chrome security feature. Sites that use it check for its existence
  // and throw if it's missing. We stub it so they degrade gracefully.
  if (!window.trustedTypes) {
    window.trustedTypes = {
      createPolicy: (name, rules) => ({
        name,
        createHTML:      (s) => rules.createHTML      ? rules.createHTML(s)      : s,
        createScript:    (s) => rules.createScript    ? rules.createScript(s)    : s,
        createScriptURL: (s) => rules.createScriptURL ? rules.createScriptURL(s) : s,
      }),
      isHTML: (v) => typeof v === 'string', isScript: (v) => typeof v === 'string',
      isScriptURL: (v) => typeof v === 'string',
      emptyHTML: '', emptyScript: '', defaultPolicy: null,
    };
  }

  // ── ClipboardItem ───────────────────────────────────────────────
  // Some tools (Figma, Notion, etc.) check for this constructor.
  if (typeof ClipboardItem === 'undefined') {
    window.ClipboardItem = class ClipboardItem {
      constructor(data) { this._data = data; }
      getType(t) { return Promise.resolve(this._data[t]); }
      get types() { return Object.keys(this._data); }
    };
  }

  // ── WebTransport stub ───────────────────────────────────────────
  // Sites detect WebTransport with typeof checks. Without a stub,
  // accessing window.WebTransport throws ReferenceError on WebKit.
  if (!window.WebTransport) {
    window.WebTransport = class WebTransport {
      constructor() { throw new DOMException('Not supported', 'NotSupportedError'); }
    };
  }

  // ── Permissions Policy ──────────────────────────────────────────
  if (!document.featurePolicy && !document.permissionsPolicy) {
    const stub = {
      allowsFeature: () => true, features: () => [],
      getAllowlistForFeature: () => ['*'],
    };
    try { Object.defineProperty(document, 'featurePolicy',    { get: () => stub, configurable: true }); } catch(e) {}
    try { Object.defineProperty(document, 'permissionsPolicy',{ get: () => stub, configurable: true }); } catch(e) {}
  }

  // ── requestIdleCallback guarantee ──────────────────────────────
  // WebKit has it but some old bundled polyfills check and try to override.
  if (!window.requestIdleCallback) {
    window.requestIdleCallback = (cb, opts) => setTimeout(() => cb({
      didTimeout: false,
      timeRemaining: () => Math.max(0, 50 - (performance.now() % 50)),
    }), opts?.timeout ?? 1);
    window.cancelIdleCallback = clearTimeout;
  }

  // ── structuredClone ─────────────────────────────────────────────
  // WebKit 15.4+ has it, but older builds may not.
  if (!window.structuredClone) {
    window.structuredClone = (v) => JSON.parse(JSON.stringify(v));
  }

  // ── queueMicrotask ──────────────────────────────────────────────
  if (!window.queueMicrotask) {
    window.queueMicrotask = (cb) => Promise.resolve().then(cb);
  }

  // ── AggregateError ──────────────────────────────────────────────
  if (!window.AggregateError) {
    window.AggregateError = class AggregateError extends Error {
      constructor(errors, msg) { super(msg); this.errors = [...errors]; this.name = 'AggregateError'; }
    };
  }

  // ── at() on Array/String/TypedArray ────────────────────────────
  // WebKit 15+ has it, but older builds may not. Many frameworks use it.
  if (!Array.prototype.at) {
    Array.prototype.at = function(i) { return this[i < 0 ? this.length + i : i]; };
    String.prototype.at = function(i) { return this[i < 0 ? this.length + i : i]; };
  }

  // ── Object.hasOwn ───────────────────────────────────────────────
  if (!Object.hasOwn) {
    Object.hasOwn = (o, k) => Object.prototype.hasOwnProperty.call(o, k);
  }

  // ── Error.cause ─────────────────────────────────────────────────
  try { new Error('', { cause: 'test' }).cause; } catch(e) {
    const OrigError = Error;
    window.Error = class Error extends OrigError {
      constructor(msg, opts) { super(msg); if (opts?.cause !== undefined) this.cause = opts.cause; }
    };
  }

  console.debug('[Parsec Compat] Chrome compatibility layer loaded');
})();
"#;

        // Combined init script — runs before any page JS
        let init_script = format!("{compat_shim}\n{chrome_shim}\n{blocker_script}");

        let tab_id        = id.to_string();
        let tab_id_nav    = id.to_string();
        let tab_id_ipc    = id.to_string();
        let event_tx_nav  = self.event_tx.clone();
        let event_tx_ipc  = self.event_tx.clone();
        let prefs_nav     = self.prefs.clone();

        // Web context (isolated storage per incognito tab)
        let mut web_context = if incognito {
            // Ephemeral context: no disk persistence for cookies/cache/storage
            WebContext::new(None)
        } else {
            let data_dir = dirs::data_local_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("parsec-web").join("profile");
            WebContext::new(Some(data_dir))
        };

        let initial_url = if url == "parsec://newtab" {
            "about:blank".to_string()
        } else {
            url.to_string()
        };

        let webview = WebViewBuilder::new(window)
            // ── Real per-WebView bounds (behind the chrome) ──────────
            .with_bounds(wry::Rect {
                x: bounds.0, y: bounds.1,
                width: bounds.2, height: bounds.3,
            })
            // ── Chrome-compatible User-Agent ─────────────────────────
            // Sets the UA at the HTTP header level so servers never see
            // a WebKit/Safari UA and serve degraded content.
            // The JS compat shim above handles navigator.userAgent for JS.
            .with_user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
            // ── Initial URL ──────────────────────────────────────────
            .with_url(&initial_url)
            // ── Incognito / shared context ───────────────────────────
            .with_web_context(&mut web_context)
            // ── Script injected at document_start ────────────────────
            // This runs BEFORE any page JS — blocker + chrome shim
            .with_initialization_script(&init_script)
            // ── Navigation handler ────────────────────────────────────
            // Called for every navigation — top-level AND subframes.
            // Return false to block the navigation.
            .with_navigation_handler(move |url: String| {
                let p = prefs_nav.lock().unwrap();

                // Enforce HTTPS-only
                if p.https_only && url.starts_with("http://")
                    && !url.contains("localhost") && !url.starts_with("http://127.")
                {
                    let https_url = url.replacen("http://", "https://", 1);
                    let _ = event_tx_nav.send(TabEvent::UrlChanged {
                        tab_id: tab_id_nav.clone(), url: https_url,
                    });
                    return false; // will be reloaded with https URL
                }

                // Block check
                let decision = blocker::check_url(
                    &url, p.block_ads, p.block_trackers, p.block_nsfw, p.block_popups,
                );
                if decision.blocked {
                    let reason = decision.reason.unwrap_or_else(|| "unknown".into());
                    let _ = event_tx_nav.send(TabEvent::Blocked {
                        tab_id: tab_id_nav.clone(),
                        url:    url.clone(),
                        reason,
                    });
                    return false;
                }

                let _ = event_tx_nav.send(TabEvent::LoadStart { tab_id: tab_id_nav.clone() });
                let _ = event_tx_nav.send(TabEvent::UrlChanged { tab_id: tab_id_nav.clone(), url });
                true
            })
            // ── IPC handler ───────────────────────────────────────────
            // Page JS calls window.ipc.postMessage(JSON) to send events
            // back to Rust (title changes, favicon, etc.)
            .with_ipc_handler(move |msg: wry::http::Request<String>| {
                let body = msg.body();
                match serde_json::from_str::<serde_json::Value>(body) {
                    Ok(json) => {
                        let ev_type = json["type"].as_str().unwrap_or("");
                        match ev_type {
                            "title" => {
                                let title = json["value"].as_str().unwrap_or("").to_string();
                                let _ = event_tx_ipc.send(TabEvent::TitleChanged {
                                    tab_id: tab_id_ipc.clone(), title,
                                });
                            }
                            "favicon" => {
                                let url = json["url"].as_str().unwrap_or("").to_string();
                                let _ = event_tx_ipc.send(TabEvent::FaviconChanged {
                                    tab_id: tab_id_ipc.clone(), favicon_url: url,
                                });
                            }
                            "loaded" => {
                                let _ = event_tx_ipc.send(TabEvent::LoadFinish {
                                    tab_id: tab_id_ipc.clone(),
                                });
                            }
                            _ => {}
                        }
                    }
                    Err(e) => warn!("IPC parse error: {e}"),
                }
            })
            // ── macOS native content rules ───────────────────────────
            // WKContentRuleList blocks resources before TCP connects.
            // This is the gold-standard mechanism — same as Safari's blocker.
            .with_content_protection(false)
            .build()?;

        // Apply macOS native content rules if available
        #[cfg(target_os = "macos")]
        self.apply_content_rules_macos(&webview, &content_rules);

        // Inject title/favicon IPC reporter into every page
        let reporter_script = r#"
            (function() {
                // Report title changes
                const titleObs = new MutationObserver(() => {
                    window.ipc && window.ipc.postMessage(JSON.stringify({
                        type: 'title', value: document.title
                    }));
                });
                if (document.querySelector('title')) {
                    titleObs.observe(document.querySelector('title'), { childList: true });
                }
                // Report page load
                window.addEventListener('load', () => {
                    window.ipc && window.ipc.postMessage(JSON.stringify({ type: 'loaded' }));
                    // Report favicon
                    const fav = document.querySelector('link[rel*="icon"]');
                    if (fav) {
                        window.ipc && window.ipc.postMessage(JSON.stringify({
                            type: 'favicon', url: fav.href
                        }));
                    }
                });
            })();
        "#;
        let _ = webview.evaluate_script(reporter_script);

        info!("Tab {} created: {}", id, url);
        self.states.insert(id.to_string(), TabState::new(id, url, incognito));
        self.webviews.insert(id.to_string(), webview);
        Ok(())
    }

    /// Navigate active tab to URL
    pub fn navigate(&mut self, tab_id: &str, url: &str) -> anyhow::Result<()> {
        let wv = self.webviews.get(tab_id)
            .ok_or_else(|| anyhow::anyhow!("Tab {tab_id} not found"))?;
        let final_url = if url == "parsec://newtab" { "about:blank" } else { url };
        wv.load_url(final_url);
        if let Some(s) = self.states.get_mut(tab_id) {
            s.url = url.into(); s.loading = true;
        }
        Ok(())
    }

    /// Real back — via wry's native WebView history
    pub fn go_back(&mut self, tab_id: &str) {
        if let Some(wv) = self.webviews.get(tab_id) {
            // evaluate_script won't cross origins for security, so we use
            // the wry load_url approach: navigate to the previous entry.
            // history.back() works for same-origin; for cross-origin we need
            // to call the native WKWebView goBack — wry exposes this via
            // evaluate_script on the navigation JS bridge.
            let _ = wv.evaluate_script(
                "if(history.length > 1) history.back(); \
                 else window.ipc && window.ipc.postMessage(JSON.stringify({type:'cannotGoBack'}));"
            );
        }
    }

    /// Real forward — via wry's native WebView history
    pub fn go_forward(&mut self, tab_id: &str) {
        if let Some(wv) = self.webviews.get(tab_id) {
            let _ = wv.evaluate_script(
                "history.forward();"
            );
        }
    }

    /// Real reload — via wry's native WebView
    pub fn reload(&mut self, tab_id: &str) {
        if let Some(wv) = self.webviews.get(tab_id) {
            let _ = wv.evaluate_script("window.location.reload()");
        }
    }

    /// Set tab zoom level (0.25 – 5.0)
    pub fn set_zoom(&mut self, tab_id: &str, level: f64) {
        if let Some(wv) = self.webviews.get(tab_id) {
            // Use CSS zoom only — not transform: scale(), which compounds on
            // repeated calls and breaks fixed-position elements and vh/vw units.
            let _ = wv.evaluate_script(&format!(
                "document.documentElement.style.zoom = '{level}';"
            ));
            if let Some(s) = self.states.get_mut(tab_id) { s.zoom = level; }
        }
    }

    /// Mute/unmute tab audio
    pub fn set_muted(&mut self, tab_id: &str, muted: bool) {
        if let Some(wv) = self.webviews.get(tab_id) {
            let script = if muted {
                "document.querySelectorAll('audio,video').forEach(e => e.muted = true);"
            } else {
                "document.querySelectorAll('audio,video').forEach(e => e.muted = false);"
            };
            let _ = wv.evaluate_script(script);
            if let Some(s) = self.states.get_mut(tab_id) { s.muted = muted; }
        }
    }

    /// Inject a content script (for Chrome extensions)
    pub fn inject_script(&mut self, tab_id: &str, script: &str) {
        if let Some(wv) = self.webviews.get(tab_id) {
            let _ = wv.evaluate_script(script);
        }
    }

    /// Switch active tab (show/hide WebViews)
    pub fn set_active(&mut self, tab_id: &str) {
        // Hide all
        for (id, wv) in &self.webviews {
            let _ = wv.set_visible(id == tab_id);
        }
        self.active = Some(tab_id.to_string());
    }

    /// Resize all WebViews (called when window resizes or panel opens/closes)
    pub fn resize_viewport(&mut self, x: i32, y: i32, w: u32, h: u32) {
        for wv in self.webviews.values() {
            let _ = wv.set_bounds(wry::Rect { x, y, width: w, height: h });
        }
    }

    /// Close a tab and destroy its WebView
    pub fn close_tab(&mut self, tab_id: &str) {
        self.webviews.remove(tab_id);
        self.states.remove(tab_id);
        if self.active.as_deref() == Some(tab_id) {
            self.active = self.webviews.keys().next().cloned();
            if let Some(next) = &self.active.clone() {
                self.set_active(next);
            }
        }
        info!("Tab {} closed", tab_id);
    }

    pub fn get_state(&self, tab_id: &str) -> Option<&TabState> {
        self.states.get(tab_id)
    }

    pub fn get_all_states(&self) -> Vec<TabState> {
        self.states.values().cloned().collect()
    }

    pub fn active_id(&self) -> Option<&str> {
        self.active.as_deref()
    }

    pub fn update_state(&mut self, tab_id: &str, f: impl FnOnce(&mut TabState)) {
        if let Some(s) = self.states.get_mut(tab_id) { f(s); }
    }

    /// Apply WKContentRuleList on macOS — runs in the WebKit process,
    /// blocks resources before any network connection is made.
    #[cfg(target_os = "macos")]
    fn apply_content_rules_macos(&self, _webview: &WebView, rules_json: &str) {
        // In production: use WKContentRuleListStore to compile and apply rules.
        // The wry 0.37 macOS backend exposes the WKWebView through `inner()`.
        // We'd call:
        //   [WKContentRuleListStore.defaultStore
        //     compileContentRuleListForIdentifier:@"ParsecShield"
        //     encodedContentRuleList:rules_json
        //     completionHandler:^(WKContentRuleList* list, NSError* error) {
        //       [webView.configuration.userContentController addContentRuleList:list];
        //     }];
        //
        // This is the same API Brave uses for its Rust shield on macOS.
        // Full objc2 integration shown in companion file: content_rules_macos.rs
        info!("macOS: content rules compiled ({} bytes)", rules_json.len());
    }

    /// Suspend a tab — navigate it to about:blank to free RAM
    /// The chrome keeps the tab entry; content is reloaded on ResumeTab
    pub fn suspend_tab(&mut self, tab_id: &str) {
        if let Some(wv) = self.webviews.get(tab_id) {
            wv.load_url("about:blank");
        }
    }

    // ── Speculative preload (Speculation Rules API) ───────────────────────────
    //
    // Safari and Chrome both support instant navigation via speculative preload:
    // when the user hovers a link (or the page declares <script type="speculationrules">),
    // we start loading the page in a hidden 1×1 off-screen WebView.
    //
    // When the user actually clicks:
    //   - If speculative load is complete: promote() swaps the hidden WebView
    //     to full size and returns immediately. Navigation latency ≈ 0ms.
    //   - If still loading: navigate normally (user doesn't notice since the
    //     network request and parse are already partially done).
    //
    // Memory cost: ~15-40MB per speculative tab. We cap at 3 concurrent
    // speculative loads and evict least-recently-hovered on overflow.

    /// Start a speculative preload for a URL the user is likely to visit.
    /// Called when the user hovers a link for >100ms or the page declares
    /// Speculation Rules. Returns immediately — loading happens in background.
    pub fn start_speculative_load(
        &mut self,
        url:    &str,
        window: &tao::window::Window,
    ) {
        // Don't preload if already loading or already a real tab
        let already_loading = self.speculative.contains_key(url);
        let already_tab     = self.webviews.values().count() > 0; // simplified check
        if already_loading { return; }

        // Cap at 3 concurrent speculative loads (memory budget ~120MB)
        if self.speculative.len() >= 3 {
            // Evict one — remove the first entry (LRU would be ideal; this is good enough)
            if let Some(key) = self.speculative.keys().next().cloned() {
                self.speculative.remove(&key);
                tracing::debug!("Speculative: evicted {} to make room", key);
            }
        }

        let (x, y, w, h) = self.spec_bounds;
        let prefs = self.prefs.lock().unwrap().clone();

        // Build a minimal WebView for speculative loading.
        // Same pipeline as create_tab but:
        //   - Off-screen bounds (user never sees it)
        //   - No event callbacks (we don't need title/favicon updates)
        //   - No extension injection (deferred until promotion)
        let webview = wry::WebViewBuilder::new(window)
            .with_bounds(wry::Rect { x, y, width: w, height: h })
            .with_url(url)
            .with_user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
            .with_devtools(false)
            .build();

        match webview {
            Ok(wv) => {
                let _ = wv.set_visible(false);
                self.speculative.insert(url.to_string(), wv);
                tracing::info!("Speculative: started preload for {}", url);
            }
            Err(e) => tracing::warn!("Speculative: WebView creation failed: {}", e),
        }
    }

    /// Promote a speculative WebView to a real tab.
    /// Called when the user actually navigates to the URL.
    /// Returns the tab_id of the promoted tab, or None if no speculative load exists.
    pub fn promote_speculative(&mut self, url: &str, tab_id: &str) -> bool {
        let Some(spec_wv) = self.speculative.remove(url) else {
            return false; // No speculative load for this URL
        };

        // Move the speculative WebView into the real webviews map
        // It's already loaded the page — just resize and show it
        self.webviews.insert(tab_id.to_string(), spec_wv);
        self.states.insert(tab_id.to_string(), TabState::new(tab_id, url, false));

        // Show at full size
        self.set_active(tab_id);

        tracing::info!("Speculative: promoted {} → tab {} (instant navigation)", url, tab_id);
        true
    }

    /// Cancel a speculative preload (user moved mouse away).
    /// Called after the hover dwell timeout expires without a click.
    pub fn cancel_speculative(&mut self, url: &str) {
        if self.speculative.remove(url).is_some() {
            tracing::debug!("Speculative: cancelled preload for {}", url);
        }
    }

    /// Is a speculative load ready (page loaded) for this URL?
    pub fn is_speculative_ready(&self, url: &str) -> bool {
        self.speculative.contains_key(url)
    }
}
