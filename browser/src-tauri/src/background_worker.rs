// src-tauri/src/background_worker.rs
//
// v1.3: Background service worker WebViews.
//
// Every installed Chrome extension with a background script (MV3 service
// worker or MV2 background page) gets a hidden wry WebView that lives for
// the entire browser session.  The WebView:
//
//   1. Loads the full chrome-compat.js shim so the background script sees
//      the complete Chrome Extension API
//   2. Executes the background script/service worker
//   3. Receives messages from content scripts via the same IPC bridge
//   4. Can inject scripts into content tabs via the inject_tx channel
//
// Architecture
// ────────────
// Background WebViews are attached to the main window, positioned off-screen
// at (-32768, -32768) with a 1×1 pixel size.  They are never visible.
// One per extension, created the first time the extension is loaded and
// destroyed when the extension is uninstalled.
//
// The event loop (main.rs) listens for SpawnBackground user events and
// creates WebViews on the main thread (the only thread where wry allows
// WebView construction on macOS/Windows).
//
// Message flow:
//   Content script → chrome.runtime.sendMessage
//     → IPC → ExtensionRuntime::dispatch(runtime, sendMessage)
//       → msg_tx → BackgroundWorkerManager::relay_message
//         → evaluate_script on background WebView
//           → window.__parsec_ext_message(msg, fromId)
//             → extension's onMessage listener fires

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use tokio::sync::mpsc;

use crate::extension_store::InstalledExtension;

// ── Background worker descriptor ──────────────────────────────────

#[derive(Debug, Clone)]
pub struct BackgroundWorker {
    pub ext_id:  String,
    pub name:    String,
    /// Path to the service worker / background script inside the extension dir
    pub script:  String,
    /// Manifest version
    pub mv:      u8,
}

// ── Events sent TO the main event loop to request WebView creation ─

#[derive(Debug)]
pub struct SpawnRequest {
    pub worker:   BackgroundWorker,
    /// Full JS to evaluate inside the background WebView at startup.
    /// = chrome-compat shim + service worker script contents
    pub init_js:  String,
}

// ── Outbound message: relay a runtime.sendMessage to a background worker

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerMessage {
    pub to_ext_id:   String,
    pub from_ext_id: String,
    pub payload:     serde_json::Value,
    pub response_id: Option<String>,
}

// ── BackgroundWorkerManager ────────────────────────────────────────

pub struct BackgroundWorkerManager {
    /// Workers we have requested creation for (may not be created yet if
    /// the main-thread event loop hasn't processed the spawn request)
    workers:   Arc<Mutex<HashMap<String, BackgroundWorker>>>,

    /// Channel to send spawn requests to the main event loop
    spawn_tx:  mpsc::UnboundedSender<SpawnRequest>,

    /// Channel to relay messages from content scripts to background workers.
    /// The main event loop drains this and calls evaluate_script on the WebView.
    relay_tx:  mpsc::UnboundedSender<WorkerMessage>,
}

impl BackgroundWorkerManager {
    pub fn new(
        spawn_tx: mpsc::UnboundedSender<SpawnRequest>,
        relay_tx: mpsc::UnboundedSender<WorkerMessage>,
    ) -> Self {
        Self {
            workers:  Arc::new(Mutex::new(HashMap::new())),
            spawn_tx,
            relay_tx,
        }
    }

    /// Called when an extension is loaded/installed.
    /// Builds the background WebView init script and sends a spawn request.
    pub fn spawn_for_extension(&self, ext: &InstalledExtension) {
        let bg_script_path = match &ext.manifest.background {
            Some(bg) => {
                if let Some(sw) = &bg.service_worker {
                    sw.clone()
                } else if let Some(first) = bg.scripts.first() {
                    first.clone()
                } else {
                    return; // no background script
                }
            }
            None => return, // no background config
        };

        // Read the script from disk
        let script_path = ext.install_path.join(&bg_script_path);
        let bg_code = match std::fs::read_to_string(&script_path) {
            Ok(c) => c,
            Err(e) => {
                warn!("Background worker {}: can't read {}: {e}", ext.id, bg_script_path);
                return;
            }
        };

        let worker = BackgroundWorker {
            ext_id: ext.id.clone(),
            name:   ext.name.clone(),
            script: bg_script_path.clone(),
            mv:     ext.manifest.manifest_version,
        };

        // Build the complete init JS
        let chrome_shim = build_background_chrome_shim(&ext.id);
        let init_js = format!(
            r#"
// ── Parsec background worker init: {name} ──────────────────────────
{chrome_shim}

// ── Extension background script ─────────────────────────────────────
(function() {{
  "use strict";
  try {{
    {bg_code}
    console.info('[Parsec BG] {name} service worker loaded');
  }} catch (e) {{
    console.error('[Parsec BG] {name} error:', e);
  }}
}})();
"#,
            name = ext.name,
            chrome_shim = chrome_shim,
            bg_code = bg_code,
        );

        info!("Spawning background worker for {} ({})", ext.name, ext.id);
        self.workers.lock().unwrap().insert(ext.id.clone(), worker.clone());

        let _ = self.spawn_tx.send(SpawnRequest { worker, init_js });
    }

    /// Relay a runtime.sendMessage to the background worker for that extension.
    pub fn send_message(&self, msg: WorkerMessage) {
        let _ = self.relay_tx.send(msg);
    }

    /// Remove a worker (called on uninstall)
    pub fn remove(&self, ext_id: &str) {
        self.workers.lock().unwrap().remove(ext_id);
    }

    pub fn has_worker(&self, ext_id: &str) -> bool {
        self.workers.lock().unwrap().contains_key(ext_id)
    }
}

// ── BackgroundWebViewSet ─────────────────────────────────────────────
// Stored on the main thread inside the event loop — holds the actual WebViews.

pub struct BackgroundWebViewSet {
    /// ext_id → WebView handle
    views: HashMap<String, wry::WebView>,
}

impl BackgroundWebViewSet {
    pub fn new() -> Self { Self { views: HashMap::new() } }

    /// Called from the main event loop when a SpawnRequest arrives.
    pub fn create(
        &mut self,
        req: SpawnRequest,
        window: &tao::window::Window,
    ) {
        if self.views.contains_key(&req.worker.ext_id) {
            return; // already created
        }

        let ext_id    = req.worker.ext_id.clone();
        let init_js   = req.init_js.clone();
        let ext_name  = req.worker.name.clone();
        let ext_id2   = ext_id.clone();

        let webview = wry::WebViewBuilder::new(window)
            // Hidden, 1×1, off-screen
            .with_bounds(wry::Rect { x: -32768, y: -32768, width: 1, height: 1 })
            .with_url("about:blank")
            .with_initialization_script(&init_js)
            // Background workers get their own IPC channel for event feedback
            .with_ipc_handler(move |body: String| {
                // Background worker sending event back (console, alarm, etc.)
                match serde_json::from_str::<serde_json::Value>(&body) {
                    Ok(json) => {
                        let ev = json["type"].as_str().unwrap_or("");
                        match ev {
                            "alarm"   => tracing::info!("BG[{ext_id2}] alarm: {}", json["name"]),
                            "console" => tracing::debug!("BG[{ext_id2}] console: {}", json["text"]),
                            _         => tracing::debug!("BG[{ext_id2}] event: {ev}"),
                        }
                    }
                    Err(_) => {}
                }
            })
            .with_devtools(false)
            .build();

        match webview {
            Ok(wv) => {
                info!("Background WebView created for {} ({})", ext_name, ext_id);
                self.views.insert(ext_id, wv);
            }
            Err(e) => {
                warn!("Background WebView creation failed for {ext_name}: {e}");
            }
        }
    }

    /// Relay a WorkerMessage to the target extension's background WebView
    pub fn relay(&mut self, msg: WorkerMessage) {
        let wv = match self.views.get_mut(&msg.to_ext_id) {
            Some(w) => w,
            None => { warn!("No background worker for {}", msg.to_ext_id); return; }
        };

        let payload_json = serde_json::to_string(&msg.payload).unwrap_or_else(|_| "null".into());
        let from_id      = serde_json::to_string(&msg.from_ext_id).unwrap_or_else(|_| "\"\"".into());
        let response_id  = serde_json::to_string(&msg.response_id).unwrap_or_else(|_| "null".into());

        let script = format!(
            r#"
(function() {{
  const msg    = {payload_json};
  const fromId = {from_id};
  const respId = {response_id};
  // Fire chrome.runtime.onMessage listeners
  if (window.chrome && window.chrome.runtime && window.chrome.runtime.onMessage) {{
    const sender = {{ id: fromId, url: 'chrome-extension://' + fromId }};
    const sendResponse = (response) => {{
      if (respId) {{
        window.__parsec_bg_send_response && window.__parsec_bg_send_response(respId, response);
      }}
    }};
    window.chrome.runtime.onMessage._fire(msg, sender, sendResponse);
  }}
}})();
"#
        );

        let _ = wv.evaluate_script(&script);
    }

    /// Evaluate arbitrary JS in an extension's background context
    pub fn evaluate(&mut self, ext_id: &str, script: &str) {
        if let Some(wv) = self.views.get_mut(ext_id) {
            let _ = wv.evaluate_script(script);
        }
    }
}

// ── Chrome shim for background workers ────────────────────────────────
//
// Background workers get the same chrome API as content scripts, but with
// a few extras:
//   - chrome.runtime.onInstalled fires at startup (simulated)
//   - Alarms actually fire via setInterval
//   - chrome.storage uses localStorage as backing (per-extension namespace)
//   - chrome.runtime.onMessage fires when relay_message is called

fn build_background_chrome_shim(ext_id: &str) -> String {
    format!(r#"
(function() {{
  "use strict";
  if (window.__parsec_bg_chrome) return;
  window.__parsec_bg_chrome = true;

  const EXT_ID = "{ext_id}";

  // ── IPC to Rust extension runtime ─────────────────────────────────
  function ipcCall(domain, method, args) {{
    return new Promise((resolve) => {{
      const id = 'bg_' + Math.random().toString(36).slice(2);
      window.__parsec_ext_replies = window.__parsec_ext_replies || {{}};
      window.__parsec_ext_replies[id] = resolve;
      if (window.ipc) {{
        window.ipc.postMessage(JSON.stringify({{
          id, cmd: 'ExtAPI',
          args: {{ extId: EXT_ID, tabId: '__background__', domain, method, args }}
        }}));
      }} else {{
        setTimeout(() => resolve(null), 10);
      }}
    }});
  }}

  // ── Event system ───────────────────────────────────────────────────
  function makeEvent() {{
    const cbs = new Set();
    return {{
      addListener:    (cb, filter) => cbs.add({{ cb, filter }}),
      removeListener: (cb) => {{ for (const e of cbs) if (e.cb === cb) cbs.delete(e); }},
      hasListener:    (cb) => [...cbs].some(e => e.cb === cb),
      _fire: (...args) => cbs.forEach(e => {{ try {{ e.cb(...args); }} catch(err) {{ console.error(err); }} }}),
    }};
  }}

  // ── Alarm store ────────────────────────────────────────────────────
  const _alarmTimers = new Map();
  const _onAlarm = makeEvent();

  function _scheduleAlarm(name, delayMs, periodMs) {{
    if (_alarmTimers.has(name)) clearTimeout(_alarmTimers.get(name).handle);
    const fire = () => {{
      _onAlarm._fire({{ name, scheduledTime: Date.now() }});
      if (periodMs > 0) {{
        const h = setInterval(() => _onAlarm._fire({{ name, scheduledTime: Date.now() }}), periodMs);
        _alarmTimers.get(name).handle = h;
      }}
    }};
    const h = delayMs > 0 ? setTimeout(fire, delayMs) : (fire(), null);
    _alarmTimers.set(name, {{ handle: h, periodMs }});
  }}

  // ── Storage backed by localStorage (namespaced) ─────────────────────
  const _ns = (area, key) => `__parsec_bg_${{EXT_ID}}_${{area}}_${{key}}`;

  function makeStorage(area) {{
    return {{
      get: (keys, cb) => {{
        const ks = typeof keys === 'string' ? [keys]
                 : Array.isArray(keys) ? keys
                 : (keys ? Object.keys(keys) : null);
        const result = {{}};
        if (ks) {{
          ks.forEach(k => {{
            const v = localStorage.getItem(_ns(area, k));
            if (v != null) result[k] = JSON.parse(v);
            else if (typeof keys === 'object' && !Array.isArray(keys) && keys[k] !== undefined)
              result[k] = keys[k];
          }});
        }} else {{
          // return all
          for (let i = 0; i < localStorage.length; i++) {{
            const raw = localStorage.key(i);
            const pfx = `__parsec_bg_${{EXT_ID}}_${{area}}_`;
            if (raw && raw.startsWith(pfx)) {{
              const k = raw.slice(pfx.length);
              result[k] = JSON.parse(localStorage.getItem(raw));
            }}
          }}
        }}
        if (cb) cb(result);
        return Promise.resolve(result);
      }},
      set: (items, cb) => {{
        Object.entries(items).forEach(([k, v]) =>
          localStorage.setItem(_ns(area, k), JSON.stringify(v)));
        if (cb) cb();
        return Promise.resolve();
      }},
      remove: (keys, cb) => {{
        const ks = Array.isArray(keys) ? keys : [keys];
        ks.forEach(k => localStorage.removeItem(_ns(area, k)));
        if (cb) cb();
        return Promise.resolve();
      }},
      clear: (cb) => {{
        const pfx = `__parsec_bg_${{EXT_ID}}_${{area}}_`;
        Object.keys(localStorage).filter(k => k.startsWith(pfx))
          .forEach(k => localStorage.removeItem(k));
        if (cb) cb();
        return Promise.resolve();
      }},
      getBytesInUse: (keys, cb) => {{ cb && cb(0); return Promise.resolve(0); }},
      onChanged: makeEvent(),
    }};
  }}

  // ── Message response shim ─────────────────────────────────────────
  window.__parsec_bg_send_response = (responseId, value) => {{
    if (window.ipc) {{
      window.ipc.postMessage(JSON.stringify({{
        id: responseId, cmd: 'ExtAPI',
        args: {{ extId: EXT_ID, tabId: '__background__',
                 domain: 'runtime', method: 'sendResponse',
                 args: {{ responseId, value }} }}
      }}));
    }}
  }};

  // ── Assemble chrome object ─────────────────────────────────────────
  const onMessage    = makeEvent();
  const onConnect    = makeEvent();
  const onInstalled  = makeEvent();
  const onStartup    = makeEvent();

  window.chrome = window.browser = {{
    runtime: {{
      id: EXT_ID,
      getManifest: () => ipcCall('runtime', 'getManifest', {{}}),
      getURL: (path) => `parsec-ext://${{EXT_ID}}/${{path}}`,
      sendMessage: (extId, msg, opts, cb) => {{
        if (typeof extId === 'object') {{ cb = opts; msg = extId; extId = EXT_ID; }}
        const p = ipcCall('runtime', 'sendMessage', {{ extensionId: extId, message: msg }});
        if (typeof cb === 'function') p.then(cb);
        return p;
      }},
      onMessage, onConnect, onInstalled, onStartup,
      onUpdateAvailable: makeEvent(),
      onSuspend: makeEvent(),
      lastError: null,
    }},
    tabs: {{
      query:       (q, cb)      => {{ const p = ipcCall('tabs','query',q);       if(cb)p.then(cb); return p; }},
      create:      (o, cb)      => {{ const p = ipcCall('tabs','create',o);      if(cb)p.then(cb); return p; }},
      update:      (id,o,cb)    => {{ const p = ipcCall('tabs','update',{{id,...o}}); if(cb)p.then(cb); return p; }},
      sendMessage: (id,m,o,cb)  => {{ if(typeof o==='function'){{cb=o;o={{}};}} const p=ipcCall('tabs','sendMessage',{{tabId:String(id),message:m,...o}}); if(cb)p.then(cb); return p; }},
      onUpdated:   makeEvent(), onCreated: makeEvent(), onRemoved: makeEvent(), onActivated: makeEvent(),
    }},
    storage: {{ local: makeStorage('local'), sync: makeStorage('sync'), session: makeStorage('session'), onChanged: makeEvent() }},
    alarms: {{
      create: (name, info, cb) => {{
        if (typeof name === 'object') {{ info = name; name = 'default'; }}
        const delay  = (info.delayInMinutes || 0) * 60_000;
        const period = (info.periodInMinutes || 0) * 60_000;
        _scheduleAlarm(name, delay, period);
        if (cb) cb();
      }},
      get:     (name, cb) => {{ const e = _alarmTimers.get(name); if(cb) cb(e ? {{name}} : undefined); return Promise.resolve(e ? {{name}} : undefined); }},
      getAll:  (cb) => {{ const all = [..._alarmTimers.keys()].map(n => ({{name: n}})); if(cb)cb(all); return Promise.resolve(all); }},
      clear:   (name, cb) => {{
        const t = _alarmTimers.get(name);
        if (t) {{ clearTimeout(t.handle); clearInterval(t.handle); _alarmTimers.delete(name); }}
        if(cb)cb(!!t); return Promise.resolve(!!t);
      }},
      clearAll:(cb) => {{
        _alarmTimers.forEach(t => {{ clearTimeout(t.handle); clearInterval(t.handle); }});
        _alarmTimers.clear(); if(cb)cb(true); return Promise.resolve(true);
      }},
      onAlarm: _onAlarm,
    }},
    notifications: {{
      create: (id, opts, cb) => {{ ipcCall('notifications','create',{{notificationId:id,options:opts}}); if(cb)cb(id||''); return Promise.resolve(id||''); }},
      clear:  (id, cb) => {{ if(cb)cb(true); return Promise.resolve(true); }},
      onClicked: makeEvent(), onClosed: makeEvent(),
    }},
    declarativeNetRequest: {{
      addDynamicRules:    (o,cb) => {{ const p=ipcCall('declarativeNetRequest','addDynamicRules',o);   if(cb)p.then(cb); return p; }},
      removeDynamicRules: (o,cb) => {{ const p=ipcCall('declarativeNetRequest','removeDynamicRules',o); if(cb)p.then(cb); return p; }},
      getDynamicRules:    (cb)   => {{ const p=ipcCall('declarativeNetRequest','getDynamicRules',{{}}); if(cb)p.then(cb); return p; }},
      updateDynamicRules: (o,cb) => {{ const p=ipcCall('declarativeNetRequest','updateDynamicRules',o); if(cb)p.then(cb); return p; }},
      isRegexSupported:   (o,cb) => {{ const p=Promise.resolve({{isSupported:true}}); if(cb)p.then(cb); return p; }},
      MAX_NUMBER_OF_DYNAMIC_AND_SESSION_RULES: 30000,
    }},
    history: {{
      search:    (q,cb) => {{ const p=ipcCall('history','search',q); if(cb)p.then(cb); return p; }},
      addUrl:    (d,cb) => {{ const p=ipcCall('history','addUrl',d); if(cb)p.then(cb); return p; }},
      deleteAll: (cb)   => {{ const p=ipcCall('history','deleteAll',{{}}); if(cb)p.then(cb); return p; }},
      onVisited: makeEvent(), onVisitRemoved: makeEvent(),
    }},
    bookmarks: {{
      search: (q,cb) => {{ const p=ipcCall('bookmarks','search',{{query:q}}); if(cb)p.then(cb); return p; }},
      create: (o,cb) => {{ const p=ipcCall('bookmarks','create',o); if(cb)p.then(cb); return p; }},
      remove: (id,cb)=> {{ const p=ipcCall('bookmarks','remove',{{id}}); if(cb)p.then(cb); return p; }},
      onCreated: makeEvent(), onRemoved: makeEvent(), onChanged: makeEvent(),
    }},
    permissions: {{
      contains: (p,cb) => {{ if(cb)cb(true); return Promise.resolve(true); }},
      request:  (p,cb) => {{ if(cb)cb(true); return Promise.resolve(true); }},
      onAdded: makeEvent(), onRemoved: makeEvent(),
    }},
    action: {{
      setBadgeText: (o,cb) => {{ ipcCall('action','setBadgeText',o); if(cb)cb(); }},
      setIcon:      (o,cb) => {{ if(cb)cb(); }},
      onClicked:    makeEvent(),
    }},
    extension: {{ getURL: (p) => `parsec-ext://${{EXT_ID}}/${{p}}`, inIncognitoContext: false }},
    i18n: {{ getMessage: (k) => k, getUILanguage: () => 'en' }},
    webNavigation: {{
      onBeforeNavigate: makeEvent(), onCommitted: makeEvent(),
      onCompleted: makeEvent(), onErrorOccurred: makeEvent(),
    }},
    webRequest: {{
      onBeforeRequest: makeEvent(), onBeforeSendHeaders: makeEvent(),
      onHeadersReceived: makeEvent(), onCompleted: makeEvent(), onErrorOccurred: makeEvent(),
    }},
  }};

  // Fire onInstalled at startup (simulates extension first-run)
  setTimeout(() => {{
    onInstalled._fire({{ reason: 'install', previousVersion: '' }});
    onStartup._fire();
    console.info('[Parsec BG] {ext_id} runtime ready');
  }}, 100);
}})();
"#, ext_id = ext_id)
}
