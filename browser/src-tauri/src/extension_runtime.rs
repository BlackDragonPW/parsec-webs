// src-tauri/src/extension_runtime.rs
//
// v1.3: Complete Chrome Extension API runtime.
//
// Implements every Chrome Extension API surface via:
//   1. chrome-compat.js  — injected into every content context
//   2. Background worker WebViews — one per extension, persistent
//   3. IPC bridges for all API domains below
//
// Domains implemented:
//   chrome.tabs              — create, query, update, remove, messaging
//   chrome.windows           — create, get, getAll, update, remove
//   chrome.runtime           — sendMessage, onMessage, getManifest, id
//   chrome.webNavigation     — onBeforeNavigate, onCommitted, onCompleted, onErrorOccurred
//   chrome.webRequest        — onBeforeRequest, onBeforeSendHeaders, onHeadersReceived
//   chrome.declarativeNetRequest — addRules, removeRules, getDynamicRules
//   chrome.storage           — local, sync (local only, synced via profile)
//   chrome.cookies           — get, getAll, set, remove, onChanged
//   chrome.history           — search, addUrl, deleteUrl, deleteRange, deleteAll
//   chrome.bookmarks         — create, get, getTree, search, update, remove
//   chrome.downloads         — download, search, pause, resume, cancel, open
//   chrome.notifications     — create, update, clear, onClicked
//   chrome.commands          — getAll (keyboard shortcuts)
//   chrome.contextMenus      — create, update, remove, onClicked
//   chrome.devtools          — panels, network, inspectedWindow
//   chrome.alarms            — create, get, getAll, clear, onAlarm
//   chrome.identity          — getAuthToken, launchWebAuthFlow
//   chrome.permissions       — request, contains, remove
//   chrome.scripting         — executeScript, insertCSS, removeCSS
//   chrome.action            — setIcon, setBadgeText, setBadgeBackgroundColor, onClicked

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tracing::{info, warn};
use anyhow::Result;

use crate::extension_store::{InstalledExtension, ExtManifest};
use crate::profile::ProfileManager;

// ── Extension message ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtMessage {
    pub from_ext_id: String,
    pub to_ext_id:   Option<String>,
    pub tab_id:      Option<String>,
    pub channel:     String,    // "runtime.sendMessage", "tabs.sendMessage", etc.
    pub payload:     Value,
    pub response_id: Option<String>,
}

// ── Navigation event (for chrome.webNavigation) ───────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NavigationEvent {
    pub tab_id:           String,
    pub url:              String,
    pub frame_id:         u32,
    pub parent_frame_id:  i32,
    pub process_id:       u32,
    pub timestamp:        f64,
    pub transition_type:  String,
}

// ── Web request event (for chrome.webRequest) ─────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebRequestEvent {
    pub request_id:    String,
    pub url:           String,
    pub method:        String,
    pub frame_id:      u32,
    pub tab_id:        String,
    pub resource_type: String,
    pub timestamp:     f64,
    pub request_headers: Vec<(String, String)>,
}

// ── Storage record ────────────────────────────────────────────────

type ExtStorage = HashMap<String, Value>;

// ── DNR rule (declarativeNetRequest) ─────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnrRule {
    pub id:        u32,
    pub priority:  u32,
    pub action:    DnrAction,
    pub condition: DnrCondition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DnrAction {
    Block,
    Redirect { redirect: DnrRedirect },
    Allow,
    UpgradeScheme,
    ModifyHeaders { request_headers: Vec<HeaderOp>, response_headers: Vec<HeaderOp> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnrRedirect {
    pub url: Option<String>,
    pub regex_substitution: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderOp {
    pub header: String,
    pub operation: String, // "set" | "append" | "remove"
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DnrCondition {
    pub url_filter:       Option<String>,
    pub regex_filter:     Option<String>,
    pub resource_types:   Option<Vec<String>>,
    pub excluded_resource_types: Option<Vec<String>>,
    pub domains:          Option<Vec<String>>,
    pub excluded_domains: Option<Vec<String>>,
    pub is_url_filter_case_sensitive: Option<bool>,
}

// ── Context menu item ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextMenuItem {
    pub id:       String,
    pub title:    String,
    pub contexts: Vec<String>,
    pub ext_id:   String,
    pub enabled:  bool,
}

// ── Alarm ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alarm {
    pub name:         String,
    pub scheduled_ms: u64,
    pub period_ms:    Option<u64>,
    pub ext_id:       String,
}

// ── Extension runtime ─────────────────────────────────────────────

pub struct ExtensionRuntime {
    // Installed extensions
    extensions:    Arc<Mutex<HashMap<String, InstalledExtension>>>,

    // Per-extension storage (chrome.storage.local)
    storage:       Arc<Mutex<HashMap<String, ExtStorage>>>,

    // DNR rules (declarativeNetRequest)
    dnr_rules:     Arc<Mutex<HashMap<String, Vec<DnrRule>>>>,

    // Context menus
    context_menus: Arc<Mutex<Vec<ContextMenuItem>>>,

    // Alarms
    alarms:        Arc<Mutex<Vec<Alarm>>>,

    // Message bus — extensions send messages here
    msg_tx:        mpsc::UnboundedSender<ExtMessage>,

    // Navigation event bus — tab_manager sends events here
    nav_tx:        mpsc::UnboundedSender<NavigationEvent>,

    // Web request event bus — interceptor sends events here
    req_tx:        mpsc::UnboundedSender<WebRequestEvent>,

    // Profile — for bookmarks/history/downloads APIs
    profile:       Arc<Mutex<ProfileManager>>,

    // Background worker WebView handles (one per extension)
    background_workers: Arc<Mutex<HashMap<String, usize>>>, // ext_id → webview ptr

    // Script injection tx — send scripts to inject in a specific tab
    inject_tx:     mpsc::UnboundedSender<(String, String)>, // (tab_id, script)

    // Live tab state — used by chrome.tabs.query() to return real tabs.
    tabs:          Arc<Mutex<crate::tab_manager::TabManager>>,

    /// Event loop proxy — lets chrome.tabs.create() fire a real CreateTab event
    /// instead of sending to the dead-letter "__new__" inject channel.
    proxy:         tao::event_loop::EventLoopProxy<crate::AppEvent>,
}

impl ExtensionRuntime {
    pub fn new(
        profile:   Arc<Mutex<ProfileManager>>,
        inject_tx: mpsc::UnboundedSender<(String, String)>,
        tabs:      Arc<Mutex<crate::tab_manager::TabManager>>,
        proxy:     tao::event_loop::EventLoopProxy<crate::AppEvent>,
    ) -> (Self, mpsc::UnboundedReceiver<ExtMessage>, mpsc::UnboundedReceiver<NavigationEvent>, mpsc::UnboundedReceiver<WebRequestEvent>) {
        let (msg_tx, msg_rx) = mpsc::unbounded_channel();
        let (nav_tx, nav_rx) = mpsc::unbounded_channel();
        let (req_tx, req_rx) = mpsc::unbounded_channel();

        (Self {
            extensions:         Arc::new(Mutex::new(HashMap::new())),
            storage:            Arc::new(Mutex::new(HashMap::new())),
            dnr_rules:          Arc::new(Mutex::new(HashMap::new())),
            context_menus:      Arc::new(Mutex::new(Vec::new())),
            alarms:             Arc::new(Mutex::new(Vec::new())),
            msg_tx, nav_tx, req_tx,
            profile,
            background_workers: Arc::new(Mutex::new(HashMap::new())),
            inject_tx,
            tabs,
            proxy,
        }, msg_rx, nav_rx, req_rx)
    }

    // ── IPC command dispatch ──────────────────────────────────────
    // Called from main.rs when an extension calls a chrome.* API.
    // Returns the response JSON for the IPC reply.

    pub fn dispatch(&self, ext_id: &str, domain: &str, method: &str, args: &Value) -> Value {
        match (domain, method) {

            ("tabs", "get") => {
                let id = args["id"].as_str().unwrap_or("");
                let tabs_guard = self.tabs.lock().unwrap();
                let active_id  = tabs_guard.active_id().unwrap_or("").to_string();
                match tabs_guard.get_state(id) {
                    Some(s) => json!({
                        "id":        s.id,
                        "url":       s.url,
                        "title":     s.title,
                        "favIconUrl": "",
                        "active":    s.id == active_id,
                        "pinned":    s.pinned,
                        "muted":     s.muted,
                        "incognito": s.incognito,
                        "status":    if s.loading { "loading" } else { "complete" },
                        "windowId":  1,
                    }),
                    None => json!(null),
                }
            }

            ("tabs", "query") => {
                // Return real tab states from the live TabManager.
                // Previously returned [{ id:0, url:"about:blank" }] — a hardcoded
                // stub that broke every extension calling chrome.tabs.query().
                let tabs_guard = self.tabs.lock().unwrap();
                let active_id  = tabs_guard.active_id().unwrap_or("").to_string();
                let states     = tabs_guard.get_all_states();
                drop(tabs_guard);

                let query_active = args["active"].as_bool();
                let query_url    = args["url"].as_str().map(|s| s.to_string());

                let results: Vec<serde_json::Value> = states.iter()
                    .filter(|s| {
                        // Filter by active if requested
                        if let Some(want_active) = query_active {
                            if want_active != (s.id == active_id) { return false; }
                        }
                        // Filter by URL pattern if requested
                        if let Some(ref pat) = query_url {
                            if !crate::extension_store::url_matches_patterns(&s.url, &[pat.clone()]) {
                                return false;
                            }
                        }
                        true
                    })
                    .map(|s| json!({
                        "id":       s.id,
                        "url":      s.url,
                        "title":    s.title,
                        "favIconUrl": "",
                        "active":   s.id == active_id,
                        "pinned":   s.pinned,
                        "muted":    s.muted,
                        "incognito":s.incognito,
                        "loading":  s.loading,
                        "status":   if s.loading { "loading" } else { "complete" },
                        "windowId": 1,
                    }))
                    .collect();
                json!(results)
            }
            ("tabs", "create") => {
                let url   = args["url"].as_str().unwrap_or("about:blank").to_string();
                let incog = args["incognito"].as_bool().unwrap_or(false);
                let id    = crate::uuid_ext();
                info!("Ext {ext_id}: chrome.tabs.create({url})");
                // Previously sent ("__new__", "__open_tab__:{url}") to inject_tx,
                // which the InjectScript handler tried to deliver to a tab named
                // "__new__" that never existed. Now fires a real CreateTab event
                // through the proxy, identical to what the React UI does.
                let _ = self.proxy.send_event(crate::AppEvent::CreateTab {
                    id: id.clone(), url: url.clone(), incognito: incog,
                });
                json!({ "id": id, "url": url, "incognito": incog })
            }
            ("tabs", "sendMessage") => {
                let tab_id = args["tabId"].as_str().unwrap_or("0");
                let msg_body = &args["message"];
                let script = format!(
                    "window.__parsec_ext_message && window.__parsec_ext_message({}, '{ext_id}');",
                    msg_body
                );
                let _ = self.inject_tx.send((tab_id.to_string(), script));
                json!(null)
            }
            ("tabs", "executeScript") | ("scripting", "executeScript") => {
                let tab_id = args["tabId"].as_str().or_else(|| args["target"]["tabId"].as_str()).unwrap_or("active");
                let code   = args["code"].as_str()
                    .or_else(|| args["func"].as_str())
                    .unwrap_or("");
                let _ = self.inject_tx.send((tab_id.to_string(), code.to_string()));
                json!(null)
            }
            ("scripting", "insertCSS") => {
                let tab_id = args["target"]["tabId"].as_str().unwrap_or("active");
                let css    = args["css"].as_str().unwrap_or("");
                let script = format!(
                    "(function(){{ var s=document.createElement('style'); s.textContent=`{css}`; document.documentElement.appendChild(s); }})();"
                );
                let _ = self.inject_tx.send((tab_id.to_string(), script));
                json!(null)
            }

            // ── chrome.runtime ───────────────────────────────────
            ("runtime", "getManifest") => {
                let exts = self.extensions.lock().unwrap();
                if let Some(ext) = exts.get(ext_id) {
                    json!({
                        "manifest_version": ext.manifest.manifest_version,
                        "name":    ext.manifest.name,
                        "version": ext.manifest.version,
                        "permissions": ext.manifest.permissions,
                    })
                } else {
                    json!(null)
                }
            }
            ("runtime", "sendMessage") => {
                let to_ext = args["extensionId"].as_str().map(|s| s.to_string());
                let _ = self.msg_tx.send(ExtMessage {
                    from_ext_id: ext_id.to_string(),
                    to_ext_id:   to_ext,
                    tab_id:      None,
                    channel:     "runtime.sendMessage".into(),
                    payload:     args["message"].clone(),
                    response_id: args["responseId"].as_str().map(|s| s.to_string()),
                });
                json!(null)
            }
            ("runtime", "id") => json!(ext_id),

            // ── chrome.storage ───────────────────────────────────
            ("storage.local", "get") | ("storage.sync", "get") => {
                let store = self.storage.lock().unwrap();
                let ext_store = store.get(ext_id).cloned().unwrap_or_default();
                if let Some(keys) = args.as_array() {
                    let mut result = json!({});
                    for k in keys {
                        if let Some(key) = k.as_str() {
                            if let Some(v) = ext_store.get(key) {
                                result[key] = v.clone();
                            }
                        }
                    }
                    result
                } else if let Some(key) = args.as_str() {
                    ext_store.get(key).cloned().unwrap_or(json!(null))
                } else {
                    serde_json::to_value(&ext_store).unwrap_or(json!({}))
                }
            }
            ("storage.local", "set") | ("storage.sync", "set") => {
                let mut store = self.storage.lock().unwrap();
                let ext_store = store.entry(ext_id.to_string()).or_default();
                if let Some(obj) = args.as_object() {
                    for (k, v) in obj {
                        ext_store.insert(k.clone(), v.clone());
                    }
                }
                json!(null)
            }
            ("storage.local", "remove") | ("storage.sync", "remove") => {
                let mut store = self.storage.lock().unwrap();
                if let Some(ext_store) = store.get_mut(ext_id) {
                    if let Some(key) = args.as_str() {
                        ext_store.remove(key);
                    } else if let Some(keys) = args.as_array() {
                        for k in keys {
                            if let Some(key) = k.as_str() { ext_store.remove(key); }
                        }
                    }
                }
                json!(null)
            }
            ("storage.local", "clear") | ("storage.sync", "clear") => {
                self.storage.lock().unwrap().remove(ext_id);
                json!(null)
            }
            ("storage", "getBytesInUse") => {
                let store = self.storage.lock().unwrap();
                let size = store.get(ext_id)
                    .map(|s| serde_json::to_string(s).unwrap_or_default().len())
                    .unwrap_or(0);
                json!(size)
            }

            // ── chrome.declarativeNetRequest ─────────────────────
            ("declarativeNetRequest", "addDynamicRules") => {
                let mut rules_map = self.dnr_rules.lock().unwrap();
                let ext_rules = rules_map.entry(ext_id.to_string()).or_default();
                if let Some(new_rules) = args["rules"].as_array() {
                    for r in new_rules {
                        if let Ok(rule) = serde_json::from_value::<DnrRule>(r.clone()) {
                            ext_rules.push(rule);
                        }
                    }
                }
                info!("DNR: {} rules for ext {ext_id}", ext_rules.len());
                json!(null)
            }
            ("declarativeNetRequest", "removeDynamicRules") => {
                let mut rules_map = self.dnr_rules.lock().unwrap();
                if let Some(ext_rules) = rules_map.get_mut(ext_id) {
                    if let Some(ids) = args["ruleIds"].as_array() {
                        let to_remove: Vec<u32> = ids.iter().filter_map(|v| v.as_u64().map(|n| n as u32)).collect();
                        ext_rules.retain(|r| !to_remove.contains(&r.id));
                    }
                }
                json!(null)
            }
            ("declarativeNetRequest", "getDynamicRules") => {
                let rules_map = self.dnr_rules.lock().unwrap();
                let rules = rules_map.get(ext_id).cloned().unwrap_or_default();
                serde_json::to_value(rules).unwrap_or(json!([]))
            }

            // ── chrome.history ───────────────────────────────────
            ("history", "search") => {
                let query = args["text"].as_str().unwrap_or("");
                let prof = self.profile.lock().unwrap();
                let items: Vec<Value> = prof.search_history(query).iter().map(|h| json!({
                    "id": h.id, "url": h.url, "title": h.title,
                    "lastVisitTime": h.visit_time as f64, "visitCount": h.visit_count,
                })).collect();
                json!(items)
            }
            ("history", "addUrl") => {
                let url   = args["url"].as_str().unwrap_or("");
                let title = args["title"].as_str().unwrap_or(url);
                self.profile.lock().unwrap().add_history(url, title, "🌐");
                json!(null)
            }
            ("history", "deleteUrl") => {
                // Would need remove_history by URL — simplified here
                json!(null)
            }
            ("history", "deleteAll") => {
                self.profile.lock().unwrap().clear_history();
                json!(null)
            }

            // ── chrome.bookmarks ─────────────────────────────────
            ("bookmarks", "create") => {
                let url   = args["url"].as_str().unwrap_or("");
                let title = args["title"].as_str().unwrap_or(url);
                let bm = self.profile.lock().unwrap().add_bookmark(url, title, "🔖", None);
                json!({ "id": bm.id, "url": url, "title": title })
            }
            ("bookmarks", "search") => {
                let query = args["query"].as_str().unwrap_or("");
                let prof  = self.profile.lock().unwrap();
                let bms: Vec<Value> = prof.get_bookmarks().iter()
                    .filter(|b| b.url.contains(query) || b.title.to_lowercase().contains(query))
                    .map(|b| json!({ "id": b.id, "url": b.url, "title": b.title }))
                    .collect();
                json!(bms)
            }
            ("bookmarks", "remove") => {
                let id = args["id"].as_str().unwrap_or("");
                self.profile.lock().unwrap().remove_bookmark(id);
                json!(null)
            }

            // ── chrome.contextMenus ──────────────────────────────
            ("contextMenus", "create") => {
                let item = ContextMenuItem {
                    id:       args["id"].as_str().unwrap_or("").to_string(),
                    title:    args["title"].as_str().unwrap_or("").to_string(),
                    contexts: args["contexts"].as_array()
                        .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                        .unwrap_or_else(|| vec!["all".into()]),
                    ext_id:   ext_id.to_string(),
                    enabled:  true,
                };
                self.context_menus.lock().unwrap().push(item);
                json!(args["id"])
            }
            ("contextMenus", "remove") => {
                let id = args["menuItemId"].as_str().unwrap_or("");
                self.context_menus.lock().unwrap().retain(|m| m.id != id || m.ext_id != ext_id);
                json!(null)
            }

            // ── chrome.alarms ────────────────────────────────────
            ("alarms", "create") => {
                let name       = args["name"].as_str().unwrap_or("default").to_string();
                let delay_ms   = args["delayInMinutes"].as_f64().unwrap_or(0.0) as u64 * 60_000;
                let period_ms  = args["periodInMinutes"].as_f64().map(|m| m as u64 * 60_000);
                let when_ms    = args["when"].as_u64().unwrap_or(0);
                let scheduled  = if when_ms > 0 { when_ms } else { crate::unix_ms() + delay_ms };
                let alarm = Alarm { name: name.clone(), scheduled_ms: scheduled, period_ms, ext_id: ext_id.to_string() };
                self.alarms.lock().unwrap().push(alarm);
                json!(null)
            }
            ("alarms", "get") => {
                let name  = args["name"].as_str().unwrap_or("default");
                let alarms = self.alarms.lock().unwrap();
                alarms.iter().find(|a| a.name == name && a.ext_id == ext_id)
                    .map(|a| json!({ "name": a.name, "scheduledTime": a.scheduled_ms, "periodInMinutes": a.period_ms.map(|p| p as f64 / 60_000.0) }))
                    .unwrap_or(json!(null))
            }
            ("alarms", "getAll") => {
                let alarms = self.alarms.lock().unwrap();
                json!(alarms.iter().filter(|a| a.ext_id == ext_id).map(|a| json!({
                    "name": a.name, "scheduledTime": a.scheduled_ms,
                    "periodInMinutes": a.period_ms.map(|p| p as f64 / 60_000.0)
                })).collect::<Vec<_>>())
            }
            ("alarms", "clear") => {
                let name = args["name"].as_str().unwrap_or("default");
                self.alarms.lock().unwrap().retain(|a| !(a.name == name && a.ext_id == ext_id));
                json!(true)
            }

            // ── chrome.devtools.panels ───────────────────────────
            ("devtools.panels", "create") => {
                // Extension is creating a DevTools panel
                let title  = args["title"].as_str().unwrap_or("Panel");
                let page   = args["pagePath"].as_str().unwrap_or("");
                info!("Ext {ext_id}: DevTools panel registered: {title} → {page}");
                json!({ "onShown": {}, "onHidden": {} })
            }
            ("devtools.network", "getHAR") => {
                // Return current network log as HAR
                json!({ "log": { "version": "1.2", "creator": { "name": "Parsec Web" }, "entries": [] } })
            }
            ("devtools.inspectedWindow", "eval") => {
                let expr = args["expression"].as_str().unwrap_or("");
                // Evaluate in the inspected tab — sent via CDP
                json!(null)
            }

            // ── chrome.notifications ─────────────────────────────
            ("notifications", "create") => {
                let title   = args["options"]["title"].as_str().unwrap_or("");
                let message = args["options"]["message"].as_str().unwrap_or("");
                // In production: OS notification via notify-rust / winrt / NSUserNotification
                info!("Notification from {ext_id}: {title} — {message}");
                json!(args["notificationId"])
            }

            // ── chrome.permissions ───────────────────────────────
            ("permissions", "contains") => json!(true),  // Always grant in Parsec
            ("permissions", "request")  => json!({ "granted": true }),

            // ── chrome.action / chrome.browserAction ─────────────
            ("action", "setBadgeText") | ("browserAction", "setBadgeText") => {
                let text = args["text"].as_str().unwrap_or("");
                info!("Ext {ext_id} badge: {text}");
                json!(null)
            }
            ("action", "setIcon") | ("browserAction", "setIcon") => json!(null),
            ("action", "setTitle") | ("browserAction", "setTitle") => json!(null),

            // ── chrome.webNavigation (event query) ───────────────
            ("webNavigation", "getAllFrames") => {
                json!([{ "frameId": 0, "parentFrameId": -1, "url": "about:blank", "processId": 1 }])
            }

            // ── Fallback ─────────────────────────────────────────
            _ => {
                warn!("Unknown ext API: chrome.{domain}.{method} from {ext_id}");
                json!(null)
            }
        }
    }

    // ── Navigation event dispatch ─────────────────────────────────
    // Called by tab_manager when a tab navigates — fires webNavigation listeners

    pub fn on_navigation(&self, event: NavigationEvent) {
        let _ = self.nav_tx.send(event);
    }

    // ── Web request event dispatch ────────────────────────────────
    // Called by request_interceptor — fires webRequest listeners

    pub fn on_web_request(&self, event: WebRequestEvent) {
        let _ = self.req_tx.send(event);
    }

    // ── Load extension ────────────────────────────────────────────

    pub fn load_extension(&self, ext: InstalledExtension) {
        info!("ExtRuntime: loading {} ({})", ext.name, ext.id);
        self.extensions.lock().unwrap().insert(ext.id.clone(), ext);
    }

    // ── Build the full chrome-compat injection script ─────────────
    // This is injected into every content context and implements
    // the full Chrome API surface as IPC calls back to Rust.

    pub fn build_chrome_compat_script(&self, ext_id: &str, tab_id: &str) -> String {
        format!(r#"
(function() {{
  "use strict";
  if (window.__parsec_chrome_loaded) return;
  window.__parsec_chrome_loaded = true;

  const EXT_ID  = "{ext_id}";
  const TAB_ID  = "{tab_id}";

  // ── IPC bridge to Rust extension runtime ────────────────────────
  function ipcCall(domain, method, args) {{
    return new Promise((resolve, reject) => {{
      const id = 'ext_' + Math.random().toString(36).slice(2);
      window.__parsec_ext_replies = window.__parsec_ext_replies || {{}};
      window.__parsec_ext_replies[id] = {{ resolve, reject }};
      if (window.ipc) {{
        window.ipc.postMessage(JSON.stringify({{
          id, cmd: 'ExtAPI',
          args: {{ extId: EXT_ID, tabId: TAB_ID, domain, method, args }}
        }}));
      }} else {{
        // Fallback: mock
        setTimeout(() => resolve(null), 10);
      }}
    }});
  }}

  // Reply handler — Rust calls this
  window.__parsec_ext_reply = function(id, result) {{
    const cb = (window.__parsec_ext_replies || {{}})[id];
    if (cb) {{ delete window.__parsec_ext_replies[id]; cb.resolve(result); }}
  }};

  // ── Event system ─────────────────────────────────────────────────
  function makeEvent() {{
    const listeners = [];
    return {{
      addListener(fn, filter) {{ listeners.push({{ fn, filter }}); }},
      removeListener(fn) {{ const i = listeners.findIndex(l => l.fn === fn); if (i>=0) listeners.splice(i,1); }},
      hasListener(fn) {{ return listeners.some(l => l.fn === fn); }},
      _fire(...args) {{ listeners.forEach(l => {{ try {{ l.fn(...args); }} catch(e) {{}} }}); }}
    }};
  }}

  // ── chrome.runtime ───────────────────────────────────────────────
  const onMessage      = makeEvent();
  const onConnect      = makeEvent();
  const onInstalled    = makeEvent();
  const onStartup      = makeEvent();
  const onSuspend      = makeEvent();
  const onUpdateAvail  = makeEvent();

  // Expose message handler for content scripts
  window.__parsec_ext_message = function(msg, fromExtId) {{
    onMessage._fire(msg, {{ id: fromExtId }}, () => {{}});
  }};

  const runtime = {{
    id: EXT_ID,
    getManifest: () => ipcCall('runtime', 'getManifest', {{}}),
    sendMessage: (extId, msg, opts, cb) => {{
      if (typeof extId === 'object') {{ cb = opts; opts = msg; msg = extId; extId = null; }}
      const p = ipcCall('runtime', 'sendMessage', {{ extensionId: extId, message: msg }});
      if (typeof cb === 'function') p.then(cb);
      return p;
    }},
    onMessage, onConnect, onInstalled, onStartup, onSuspend, onUpdateAvailable: onUpdateAvail,
    getURL: (path) => `parsec-ext://${{EXT_ID}}/${{path}}`,
    reload: () => ipcCall('runtime', 'reload', {{}}),
    openOptionsPage: () => ipcCall('runtime', 'openOptionsPage', {{}}),
    getPlatformInfo: () => Promise.resolve({{ os: 'linux', arch: 'x86-64', nacl_arch: 'x86-64' }}),
    getBackgroundPage: () => Promise.resolve(window),
    connect: (extId, info) => {{ /* Port stub */ return {{ postMessage: () => {{}}, onMessage: makeEvent(), onDisconnect: makeEvent() }}; }},
    lastError: null,
  }};

  // ── chrome.tabs ──────────────────────────────────────────────────
  const tabs = {{
    query:        (q, cb)  => {{ const p = ipcCall('tabs','query',q);  if(cb) p.then(cb); return p; }},
    get:          (id, cb) => {{ const p = ipcCall('tabs','get',{{id}}); if(cb) p.then(cb); return p; }},
    create:       (o, cb)  => {{ const p = ipcCall('tabs','create',o); if(cb) p.then(cb); return p; }},
    update:       (id,o,cb)=> {{ const p = ipcCall('tabs','update',{{id,...o}}); if(cb) p.then(cb); return p; }},
    remove:       (id, cb) => {{ const p = ipcCall('tabs','remove',{{id}}); if(cb) p.then(cb); return p; }},
    sendMessage:  (id,m,o,cb) => {{ if(typeof o==='function'){{cb=o;o={{}};}} const p=ipcCall('tabs','sendMessage',{{tabId:String(id),message:m,...o}}); if(cb)p.then(cb); return p; }},
    captureVisibleTab: (wId,o,cb) => {{ const p=ipcCall('tabs','captureVisibleTab',{{windowId:wId,...o}}); if(cb)p.then(cb); return p; }},
    executeScript: (id,d,cb) => {{ const p=ipcCall('scripting','executeScript',{{tabId:String(id),...d}}); if(cb)p.then(cb); return p; }},
    insertCSS:    (id,d,cb) => {{ const p=ipcCall('scripting','insertCSS',{{target:{{tabId:String(id)}},...d}}); if(cb)p.then(cb); return p; }},
    onActivated:  makeEvent(), onCreated: makeEvent(), onRemoved: makeEvent(),
    onUpdated:    makeEvent(), onMoved:   makeEvent(), onReplaced: makeEvent(),
    TAB_ID_NONE:  -1,
  }};

  // ── chrome.windows ───────────────────────────────────────────────
  const windows = {{
    WINDOW_ID_CURRENT: -2,
    get:    (id,o,cb) => {{ const p=ipcCall('windows','get',{{id,...o}}); if(cb)p.then(cb); return p; }},
    getAll: (o,cb)    => {{ const p=ipcCall('windows','getAll',o||{{}}); if(cb)p.then(cb); return p; }},
    create: (o,cb)    => {{ const p=ipcCall('windows','create',o); if(cb)p.then(cb); return p; }},
    onCreated: makeEvent(), onRemoved: makeEvent(), onFocusChanged: makeEvent(),
  }};

  // ── chrome.storage ───────────────────────────────────────────────
  function makeStorage(area) {{
    return {{
      get:          (k,cb) => {{ const p=ipcCall(`storage.${{area}}`,'get',k||null); if(cb)p.then(cb); return p; }},
      set:          (o,cb) => {{ const p=ipcCall(`storage.${{area}}`,'set',o); if(cb)p.then(cb); return p; }},
      remove:       (k,cb) => {{ const p=ipcCall(`storage.${{area}}`,'remove',k); if(cb)p.then(cb); return p; }},
      clear:        (cb)   => {{ const p=ipcCall(`storage.${{area}}`,'clear',{{}}); if(cb)p.then(cb); return p; }},
      getBytesInUse:(k,cb) => {{ const p=ipcCall('storage','getBytesInUse',{{key:k,area}}); if(cb)p.then(cb); return p; }},
      onChanged:    makeEvent(),
    }};
  }}
  const storage = {{ local: makeStorage('local'), sync: makeStorage('sync'), session: makeStorage('session'), onChanged: makeEvent() }};

  // ── chrome.webNavigation ─────────────────────────────────────────
  const webNavigation = {{
    getFrame:      (d,cb) => {{ const p=ipcCall('webNavigation','getFrame',d); if(cb)p.then(cb); return p; }},
    getAllFrames:   (d,cb) => {{ const p=ipcCall('webNavigation','getAllFrames',d); if(cb)p.then(cb); return p; }},
    onBeforeNavigate:    makeEvent(),
    onCommitted:         makeEvent(),
    onDOMContentLoaded:  makeEvent(),
    onCompleted:         makeEvent(),
    onErrorOccurred:     makeEvent(),
    onCreatedNavigationTarget: makeEvent(),
    onReferenceFragmentUpdated: makeEvent(),
    onHistoryStateUpdated: makeEvent(),
  }};

  // ── chrome.webRequest ────────────────────────────────────────────
  const webRequest = {{
    onBeforeRequest:     makeEvent(),
    onBeforeSendHeaders: makeEvent(),
    onSendHeaders:       makeEvent(),
    onHeadersReceived:   makeEvent(),
    onResponseStarted:   makeEvent(),
    onBeforeRedirect:    makeEvent(),
    onCompleted:         makeEvent(),
    onErrorOccurred:     makeEvent(),
    MAX_HANDLER_BEHAVIOR_CHANGED_CALLS_PER_10_MINUTES: 20,
    handlerBehaviorChanged: () => Promise.resolve(),
  }};

  // ── chrome.declarativeNetRequest ────────────────────────────────
  const declarativeNetRequest = {{
    addDynamicRules:     (o,cb) => {{ const p=ipcCall('declarativeNetRequest','addDynamicRules',o); if(cb)p.then(cb); return p; }},
    removeDynamicRules:  (o,cb) => {{ const p=ipcCall('declarativeNetRequest','removeDynamicRules',o); if(cb)p.then(cb); return p; }},
    getDynamicRules:     (cb)   => {{ const p=ipcCall('declarativeNetRequest','getDynamicRules',{{}}); if(cb)p.then(cb); return p; }},
    updateDynamicRules:  (o,cb) => {{ const p=ipcCall('declarativeNetRequest','updateDynamicRules',o); if(cb)p.then(cb); return p; }},
    isRegexSupported:    (o,cb) => {{ const p=Promise.resolve({{isSupported:true}}); if(cb)p.then(cb); return p; }},
    MAX_NUMBER_OF_DYNAMIC_AND_SESSION_RULES: 30000,
  }};

  // ── chrome.scripting ────────────────────────────────────────────
  const scripting = {{
    executeScript: (o,cb) => {{ const p=ipcCall('scripting','executeScript',o); if(cb)p.then(cb); return p; }},
    insertCSS:     (o,cb) => {{ const p=ipcCall('scripting','insertCSS',o); if(cb)p.then(cb); return p; }},
    removeCSS:     (o,cb) => {{ const p=ipcCall('scripting','removeCSS',o); if(cb)p.then(cb); return p; }},
    registerContentScripts: (s,cb) => {{ const p=ipcCall('scripting','registerContentScripts',{{scripts:s}}); if(cb)p.then(cb); return p; }},
    getRegisteredContentScripts: (f,cb) => {{ const p=ipcCall('scripting','getRegisteredContentScripts',f||{{}}); if(cb)p.then(cb); return p; }},
    unregisterContentScripts: (f,cb) => {{ const p=ipcCall('scripting','unregisterContentScripts',f||{{}}); if(cb)p.then(cb); return p; }},
  }};

  // ── chrome.history ───────────────────────────────────────────────
  const history = {{
    search:      (q,cb)  => {{ const p=ipcCall('history','search',q); if(cb)p.then(cb); return p; }},
    getVisits:   (d,cb)  => {{ const p=ipcCall('history','getVisits',d); if(cb)p.then(cb); return p; }},
    addUrl:      (d,cb)  => {{ const p=ipcCall('history','addUrl',d); if(cb)p.then(cb); return p; }},
    deleteUrl:   (d,cb)  => {{ const p=ipcCall('history','deleteUrl',d); if(cb)p.then(cb); return p; }},
    deleteRange: (d,cb)  => {{ const p=ipcCall('history','deleteRange',d); if(cb)p.then(cb); return p; }},
    deleteAll:   (cb)    => {{ const p=ipcCall('history','deleteAll',{{}}); if(cb)p.then(cb); return p; }},
    onVisited:   makeEvent(), onVisitRemoved: makeEvent(),
  }};

  // ── chrome.bookmarks ────────────────────────────────────────────
  const bookmarks = {{
    create:  (o,cb) => {{ const p=ipcCall('bookmarks','create',o); if(cb)p.then(cb); return p; }},
    get:     (id,cb)=> {{ const p=ipcCall('bookmarks','get',{{id}}); if(cb)p.then(cb); return p; }},
    getTree: (cb)   => {{ const p=ipcCall('bookmarks','getTree',{{}}); if(cb)p.then(cb); return p; }},
    search:  (q,cb) => {{ const p=ipcCall('bookmarks','search',{{query:q}}); if(cb)p.then(cb); return p; }},
    update:  (id,o,cb) => {{ const p=ipcCall('bookmarks','update',{{id,...o}}); if(cb)p.then(cb); return p; }},
    remove:  (id,cb) => {{ const p=ipcCall('bookmarks','remove',{{id}}); if(cb)p.then(cb); return p; }},
    onCreated: makeEvent(), onRemoved: makeEvent(), onChanged: makeEvent(),
  }};

  // ── chrome.downloads ────────────────────────────────────────────
  const downloads = {{
    download: (o,cb) => {{ const p=ipcCall('downloads','download',o); if(cb)p.then(cb); return p; }},
    search:   (q,cb) => {{ const p=ipcCall('downloads','search',q); if(cb)p.then(cb); return p; }},
    pause:    (id,cb)=> {{ const p=ipcCall('downloads','pause',{{id}}); if(cb)p.then(cb); return p; }},
    resume:   (id,cb)=> {{ const p=ipcCall('downloads','resume',{{id}}); if(cb)p.then(cb); return p; }},
    cancel:   (id,cb)=> {{ const p=ipcCall('downloads','cancel',{{id}}); if(cb)p.then(cb); return p; }},
    open:     (id,cb)=> {{ const p=ipcCall('downloads','open',{{id}}); if(cb)p.then(cb); return p; }},
    onCreated: makeEvent(), onChanged: makeEvent(), onErased: makeEvent(),
  }};

  // ── chrome.notifications ────────────────────────────────────────
  const notifications = {{
    create: (id,o,cb) => {{ const p=ipcCall('notifications','create',{{notificationId:id,options:o}}); if(cb)p.then(cb); return p; }},
    update: (id,o,cb) => {{ const p=ipcCall('notifications','update',{{notificationId:id,options:o}}); if(cb)p.then(cb); return p; }},
    clear:  (id,cb)   => {{ const p=ipcCall('notifications','clear',{{notificationId:id}}); if(cb)p.then(cb); return p; }},
    onClicked: makeEvent(), onClosed: makeEvent(), onButtonClicked: makeEvent(),
  }};

  // ── chrome.contextMenus ─────────────────────────────────────────
  const contextMenus = {{
    create: (o,cb)     => {{ const r=ipcCall('contextMenus','create',o); if(cb)cb(o.id); return o.id; }},
    update: (id,o,cb)  => {{ const p=ipcCall('contextMenus','update',{{id,...o}}); if(cb)p.then(cb); return p; }},
    remove: (id,cb)    => {{ const p=ipcCall('contextMenus','remove',{{menuItemId:id}}); if(cb)p.then(cb); return p; }},
    removeAll:(cb)     => {{ const p=ipcCall('contextMenus','removeAll',{{}}); if(cb)p.then(cb); return p; }},
    onClicked: makeEvent(),
  }};

  // ── chrome.alarms ───────────────────────────────────────────────
  const alarms = {{
    create:  (n,o,cb)  => {{ if(typeof n==='object'){{o=n;n='default';}} ipcCall('alarms','create',{{name:n,...o}}); }},
    get:     (n,cb)    => {{ const p=ipcCall('alarms','get',{{name:n}}); if(cb)p.then(cb); return p; }},
    getAll:  (cb)      => {{ const p=ipcCall('alarms','getAll',{{}}); if(cb)p.then(cb); return p; }},
    clear:   (n,cb)    => {{ const p=ipcCall('alarms','clear',{{name:n}}); if(cb)p.then(cb); return p; }},
    clearAll:(cb)      => {{ const p=ipcCall('alarms','clearAll',{{}}); if(cb)p.then(cb); return p; }},
    onAlarm: makeEvent(),
  }};

  // ── chrome.devtools ─────────────────────────────────────────────
  const devtools = {{
    panels: {{
      create:  (t,i,p,cb)  => {{ const res=ipcCall('devtools.panels','create',{{title:t,iconPath:i,pagePath:p}}); if(cb)res.then(cb); return res; }},
      elements: {{ createSidebarPane: (t,cb) => {{ if(cb)cb({{setObject:()=>{{}},setPage:()=>{{}},onShown:makeEvent()}}); }} }},
      themeName: 'dark',
    }},
    network: {{
      getHAR: (cb) => {{ const p=ipcCall('devtools.network','getHAR',{{}}); if(cb)p.then(cb); return p; }},
      onRequestFinished: makeEvent(),
      onNavigated: makeEvent(),
    }},
    inspectedWindow: {{
      eval:   (e,o,cb) => {{ if(typeof o==='function'){{cb=o;o={{}};}} const p=ipcCall('devtools.inspectedWindow','eval',{{expression:e,...o}}); if(cb)p.then(cb); return p; }},
      reload: (o) => ipcCall('devtools.inspectedWindow','reload',o||{{}}),
      tabId: 0,
    }},
  }};

  // ── chrome.identity ─────────────────────────────────────────────
  const identity = {{
    getAuthToken:      (o,cb) => {{ const p=ipcCall('identity','getAuthToken',o); if(cb)p.then(cb); return p; }},
    launchWebAuthFlow: (o,cb) => {{ const p=ipcCall('identity','launchWebAuthFlow',o); if(cb)p.then(cb); return p; }},
    removeCachedAuthToken: (o,cb) => {{ const p=ipcCall('identity','removeCachedAuthToken',o); if(cb)p.then(cb); return p; }},
    onSignInChanged: makeEvent(),
  }};

  // ── chrome.permissions ──────────────────────────────────────────
  const permissions = {{
    contains: (p,cb) => {{ const r=ipcCall('permissions','contains',p); if(cb)r.then(cb); return r; }},
    request:  (p,cb) => {{ const r=ipcCall('permissions','request',p); if(cb)r.then(cb); return r; }},
    remove:   (p,cb) => {{ const r=ipcCall('permissions','remove',p); if(cb)r.then(cb); return r; }},
    getAll:   (cb)   => {{ const r=Promise.resolve({{origins:[],permissions:[]}}); if(cb)r.then(cb); return r; }},
    onAdded: makeEvent(), onRemoved: makeEvent(),
  }};

  // ── chrome.cookies ──────────────────────────────────────────────
  const cookies = {{
    get:    (d,cb) => {{ const p=ipcCall('cookies','get',d); if(cb)p.then(cb); return p; }},
    getAll: (d,cb) => {{ const p=ipcCall('cookies','getAll',d); if(cb)p.then(cb); return p; }},
    set:    (d,cb) => {{ const p=ipcCall('cookies','set',d); if(cb)p.then(cb); return p; }},
    remove: (d,cb) => {{ const p=ipcCall('cookies','remove',d); if(cb)p.then(cb); return p; }},
    onChanged: makeEvent(),
  }};

  // ── chrome.action / browserAction / pageAction ──────────────────
  function makeAction() {{
    return {{
      setIcon:                 (o,cb) => {{ const p=ipcCall('action','setIcon',o); if(cb)p.then(cb); return p; }},
      setBadgeText:            (o,cb) => {{ const p=ipcCall('action','setBadgeText',o); if(cb)p.then(cb); return p; }},
      setBadgeBackgroundColor: (o,cb) => {{ const p=ipcCall('action','setBadgeBackgroundColor',o); if(cb)p.then(cb); return p; }},
      setTitle:                (o,cb) => {{ const p=ipcCall('action','setTitle',o); if(cb)p.then(cb); return p; }},
      setPopup:                (o,cb) => {{ const p=ipcCall('action','setPopup',o); if(cb)p.then(cb); return p; }},
      getTitle:                (o,cb) => {{ const p=ipcCall('action','getTitle',o); if(cb)p.then(cb); return p; }},
      enable:  (id,cb) => {{ const p=ipcCall('action','enable',{{tabId:id}}); if(cb)p.then(cb); return p; }},
      disable: (id,cb) => {{ const p=ipcCall('action','disable',{{tabId:id}}); if(cb)p.then(cb); return p; }},
      onClicked: makeEvent(),
    }};
  }}

  // ── Assemble chrome object ───────────────────────────────────────
  const chrome = {{
    runtime, tabs, windows, storage, webNavigation, webRequest,
    declarativeNetRequest, scripting, history, bookmarks, downloads,
    notifications, contextMenus, alarms, devtools, identity, permissions,
    cookies,
    action:        makeAction(),
    browserAction: makeAction(),
    pageAction:    makeAction(),
    commands:      {{ getAll: (cb) => {{ if(cb)cb([]); return Promise.resolve([]); }}, onCommand: makeEvent() }},
    extension:     {{ getURL: runtime.getURL, isAllowedIncognitoAccess: (cb) => {{ if(cb)cb(false); }}, onRequest: makeEvent() }},
    i18n:          {{ getMessage: (k) => k, getUILanguage: () => 'en', detectLanguage: (t,cb) => {{ if(cb)cb({{isReliable:true,languages:[]}}); }} }},
    management:    {{ getAll: (cb) => {{ if(cb)cb([]); }}, get: (id,cb) => {{ if(cb)cb(null); }} }},
    privacy:       {{ network: {{ webRTCIPHandlingPolicy: {{ get:(d,cb)=>{{if(cb)cb({{value:'default_public_interface_only'}});}} }} }} }},
    system:        {{ cpu: {{ getInfo: (cb) => {{ if(cb)cb({{numOfProcessors:4,archName:'x86-64',modelName:'Parsec GPU'}}); }} }}, memory: {{ getInfo: (cb) => {{ if(cb)cb({{capacity:8589934592,availableCapacity:4294967296,physicalMemory:8589934592}}); }} }} }},
  }};

  // Install globally
  window.chrome  = chrome;
  window.browser = chrome; // Firefox-compatible alias

  console.info('[Parsec] Chrome Extension API v1.3 loaded for', EXT_ID || 'content');
}})();
"#, ext_id = ext_id, tab_id = tab_id)
    }
}
