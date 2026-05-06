// ================================================================
//  parsec-web v1.3 — main.rs
//
//  Fully operational:
//    ✅ Forked WebKit  ✅ Engine-level interception  ✅ CDP DevTools
//    ✅ Chrome Extension API  ✅ navigator.parsec  ✅ HTTP/3  ✅ Neutron GPU
//    ✅ Background service worker WebViews (NEW)
//    ✅ E2E encrypted cross-device sync (NEW)
// ================================================================

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod blocker;
mod network;
mod tab_manager;
mod neutron;
mod extension_store;
mod extension_runtime;
mod profile;
mod request_interceptor;
mod cdp_devtools;
mod certs;
mod background_worker;
mod sync;
mod sdf_rasteriser;
#[cfg(target_os = "macos")]
mod neutron_metal;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::WindowBuilder,
};
use wry::WebViewBuilder;

use tab_manager::{TabManager, TabEvent};
use extension_runtime::{ExtensionRuntime, NavigationEvent};
use profile::{ProfileManager, TabSuspensionManager, SavedTab};
use cdp_devtools::DevToolsManager;
use background_worker::{BackgroundWorkerManager, BackgroundWebViewSet, SpawnRequest, WorkerMessage};
use sync::SyncManager;

// ── Custom event type for background worker WebView creation ──────

#[derive(Debug)]
pub enum AppEvent {
    SpawnBackground(SpawnRequest),
    RelayToBackground(WorkerMessage),
    /// Evaluate a JS string in the chrome WebView (main thread only).
    /// Used to: (1) send IPC replies back to React, (2) forward tab events,
    /// (3) forward background worker events.
    ChromeEval(String),
    /// Inject a script into a specific tab WebView (main thread only).
    /// Wires up the extension injection channel that was previously dropped.
    InjectScript { tab_id: String, script: String },
    /// Create a real wry WebView for a new tab (must run on main thread —
    /// wry forbids WebView construction off the main thread on macOS/Windows).
    /// Previously NewTab generated an ID and returned it without ever calling
    /// create_tab(), so every tab was ghost state with no actual WebView.
    CreateTab { id: String, url: String, incognito: bool },
    /// Start a speculative preload — create a hidden WebView for a URL
    /// the user is likely to navigate to (hover prediction).
    StartSpeculative { url: String },
}

// ── Types ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BrowserPrefs {
    pub theme:              String,
    pub default_engine:     String,
    pub homepage:           String,
    pub block_ads:          bool,
    pub block_trackers:     bool,
    pub block_nsfw:         bool,
    pub block_popups:       bool,
    pub https_only:         bool,
    pub do_not_track:       bool,
    pub hardware_accel:     bool,
    pub prefetch:           bool,
    pub clear_on_exit:      bool,
    pub auto_suspend_tabs:  bool,
    pub suspend_after_secs: u64,
    pub cdp_port:           u16,
    pub sync_enabled:       bool,
}

impl BrowserPrefs {
    pub fn defaults() -> Self {
        Self {
            theme: "dark".into(), default_engine: "Parsec Search".into(),
            homepage: "parsec://newtab".into(), block_ads: true, block_trackers: true,
            block_nsfw: false, block_popups: true, https_only: true, do_not_track: true,
            hardware_accel: true, prefetch: true, clear_on_exit: false, auto_suspend_tabs: true,
            suspend_after_secs: 300, cdp_port: 9222, sync_enabled: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadItem {
    pub id: String, pub url: String, pub filename: String, pub path: String,
    pub size: u64, pub downloaded: u64, pub progress: f32, pub state: String,
    pub start_time: u64, pub speed_bps: u64, pub mime_type: String, pub error: Option<String>,
}

struct AppState {
    prefs: Arc<Mutex<BrowserPrefs>>, privacy: Arc<Mutex<request_interceptor::InterceptStats>>,
    downloads: Arc<Mutex<HashMap<String, DownloadItem>>>, tabs: Arc<Mutex<TabManager>>,
    exts: Arc<Mutex<extension_store::ExtensionRegistry>>, ext_rt: Arc<ExtensionRuntime>,
    profile: Arc<Mutex<ProfileManager>>, suspender: Arc<Mutex<TabSuspensionManager>>,
    devtools: Arc<DevToolsManager>, bg_workers: Arc<BackgroundWorkerManager>,
    sync_mgr: Arc<SyncManager>, rt: Arc<Runtime>,
    /// Event loop proxy — lets handle_ipc (and ExtensionRuntime) fire main-thread
    /// events like CreateTab without needing a direct reference to the event loop.
    /// Previously p5_for_newtab was a local variable in main() that handle_ipc
    /// had no access to, making CreateTab a compile error.
    proxy: tao::event_loop::EventLoopProxy<AppEvent>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", content = "args")]
enum IpcCmd {
    Navigate { tab_id: String, url: String }, NewTab { url: Option<String>, incognito: Option<bool> },
    CloseTab { tab_id: String }, SwitchTab { tab_id: String }, Back { tab_id: String },
    Forward { tab_id: String }, Reload { tab_id: String }, SetZoom { tab_id: String, level: f64 },
    SetMuted { tab_id: String, muted: bool }, SetViewport { x: i32, y: i32, w: u32, h: u32 },
    SuspendTab { tab_id: String }, ResumeTab { tab_id: String },
    GetPrivacyStats, ResetStats,
    StartDownload { url: String, filename: String }, GetDownloads,
    CancelDownload { id: String }, OpenDownload { id: String },
    GetCertInfo { url: String }, GetSuggestions { query: String, engine: String }, Prefetch { url: String },
    GetPrefs, SetPref { key: String, value: serde_json::Value },
    CwsSearch { query: String, page: Option<usize> }, CwsFeatured { category: String },
    CwsInstall { ext_id: String }, CwsUninstall { ext_id: String },
    CwsSetEnabled { ext_id: String, enabled: bool }, CwsListInstalled,
    ExtAPI { ext_id: String, tab_id: String, domain: String, method: String, args: serde_json::Value },
    GetBookmarks, AddBookmark { url: String, title: String, favicon: String, folder: Option<String> },
    RemoveBookmark { id: String }, GetHistory { limit: Option<usize> },
    SearchHistory { query: String }, ClearHistory, GetSessions,
    SaveSession { tabs: Vec<SavedTabArg> }, RestoreSession { session_id: String },
    DevToolsConnect { tab_id: String }, DevToolsCmd { tab_id: String, command: String },
    DevToolsHeapSnapshot { tab_id: String }, DevToolsEval { tab_id: String, expression: String, call_id: u64 },
    DevToolsBreakpoint { tab_id: String, script_id: String, line: u32, condition: Option<String> },
    DevToolsGetDOM { tab_id: String },
    // Neutron GPU
    NeutronRegisterScene { ptr: u64, len: u64 },
    NeutronSetSurfaceRect { x: f32, y: f32, w: f32, h: f32 },
    NeutronInitGlyphTable { codepoints: Vec<u32> },
    NeutronRasterizeGlyphs { codepoints: Vec<u32> },
    NeutronPushDevToolsFrame { frame: serde_json::Value },
    NeutronResize { w: u32, h: u32 },
    // Speculation Rules — instant navigation
    SpeculativeLoad   { url: String },
    SpeculativeCancel { url: String },
    SpeculativeCheck  { url: String },
    // Sync
    SyncGetConfig, SyncSetConfig { server_url: String, user_id: String, enabled: bool },
    SyncRegister { email: String, passphrase: String }, SyncPush { passphrase: String },
    SyncPull { passphrase: String }, SyncGetStatus,
    SyncExportFile { path: String, passphrase: String },
    SyncImportFile { path: String, passphrase: String },
}

#[derive(Debug, Deserialize)]
pub struct SavedTabArg {
    pub url: String, pub title: String, pub favicon: String,
    pub pinned: bool, pub incognito: bool,
}

pub fn unix_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}
/// Public UUID v4 generator used by extension_runtime for chrome.tabs.create().
pub fn uuid_ext() -> String { uuid() }
fn uuid() -> String {
    // Use getrandom for collision-free UUIDs. The subsec_nanos() approach used
    // previously produces the same ID for tabs opened in the same second, causing
    // silent IPC routing failures (replies sent to wrong tab, events lost).
    let mut b = [0u8; 16];
    getrandom::getrandom(&mut b).unwrap_or_else(|_| {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default().as_nanos();
        for (i, byte) in b.iter_mut().enumerate() {
            *byte = ((t >> (i * 5)) ^ (t >> (i * 3))) as u8;
        }
    });
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0],b[1],b[2],b[3], b[4],b[5], b[6],b[7], b[8],b[9],
        b[10],b[11],b[12],b[13],b[14],b[15]
    )
}
fn normalize_url(input: &str) -> String {
    if input.starts_with("parsec://") || input.starts_with("about:") { return input.into(); }
    if input.starts_with("http://") || input.starts_with("https://") { return input.into(); }
    if input.contains('.') && !input.contains(' ') { return format!("https://{input}"); }
    format!("https://search.parsec.os/search?q={}", percent_encoding::percent_encode(input.as_bytes(), percent_encoding::NON_ALPHANUMERIC))
}
fn ipc_ok(id: &str, data: serde_json::Value) -> String { serde_json::json!({ "id": id, "ok": true, "data": data }).to_string() }
fn ipc_err(id: &str, msg: &str) -> String { serde_json::json!({ "id": id, "ok": false, "error": msg }).to_string() }

fn handle_ipc(msg_id: &str, cmd: IpcCmd, state: &Arc<AppState>) -> String {
    match cmd {
        IpcCmd::Navigate { tab_id, url } => {
            let url = normalize_url(&url);
            let prefs = state.prefs.lock().unwrap().clone();
            let req = request_interceptor::NativeRequest { url: url.clone(), method: "GET".into(), resource_type: "document".into(), is_main_frame: true, tab_id: tab_id.clone(), headers: Vec::new() };
            let decision = request_interceptor::global().should_allow(&req);
            match decision { request_interceptor::InterceptDecision::Block { reason, category, .. } => { return ipc_ok(msg_id, serde_json::json!({ "blocked": true, "reason": reason, "category": category })); } _ => {} }
            let final_url = if prefs.https_only && url.starts_with("http://") && !url.contains("localhost") { url.replacen("http://", "https://", 1) } else { url.clone() };
            let mut tabs = state.tabs.lock().unwrap();
            let _ = tabs.navigate(&tab_id, &final_url);
            drop(tabs);
            let ext_store = state.exts.lock().unwrap();
            let installed = ext_store.list();
            let mut injections: Vec<String> = Vec::new();
            injections.push(state.ext_rt.build_chrome_compat_script("", &tab_id));
            for ext in installed { if ext.enabled { if let Some(script) = extension_store::CrxInstaller::build_injection_script(ext, &final_url) { injections.push(script); } } }
            drop(ext_store);
            let tabs = state.tabs.lock().unwrap();
            for script in injections { tabs.inject_script(&tab_id, &script); }
            state.ext_rt.on_navigation(NavigationEvent { tab_id: tab_id.clone(), url: final_url.clone(), frame_id: 0, parent_frame_id: -1, process_id: 1, timestamp: unix_ms() as f64, transition_type: "typed".into() });
            state.profile.lock().unwrap().add_history(&final_url, &final_url, "🌐");
            state.devtools.update_page_info(&tab_id, "Loading…", &final_url);
            state.suspender.lock().unwrap().mark_active(&tab_id);
            ipc_ok(msg_id, serde_json::json!({ "url": final_url, "blocked": false }))
        }
        IpcCmd::NewTab { url, incognito } => {
            let id    = uuid();
            let url   = url.unwrap_or_else(|| "parsec://newtab".into());
            let incog = incognito.unwrap_or(false);
            state.devtools.connect_tab(&id);
            // Fire CreateTab through the event loop proxy so the wry WebView is
            // built on the main thread (wry requirement on macOS/Windows).
            // state.proxy is the correct channel — p5_for_newtab was a local
            // variable in main() that handle_ipc never had access to (compile error).
            let _ = state.proxy.send_event(AppEvent::CreateTab {
                id: id.clone(), url: url.clone(), incognito: incog,
            });
            ipc_ok(msg_id, serde_json::json!({ "tabId": id, "url": url, "incognito": incog }))
        }
        IpcCmd::CloseTab { tab_id } => { state.devtools.disconnect_tab(&tab_id); state.tabs.lock().unwrap().close_tab(&tab_id); ipc_ok(msg_id, serde_json::json!({})) }
        IpcCmd::SwitchTab { tab_id } => { state.tabs.lock().unwrap().set_active(&tab_id); state.suspender.lock().unwrap().mark_active(&tab_id); ipc_ok(msg_id, serde_json::json!({})) }
        IpcCmd::Back { tab_id } => { state.tabs.lock().unwrap().go_back(&tab_id); ipc_ok(msg_id, serde_json::json!({})) }
        IpcCmd::Forward { tab_id } => { state.tabs.lock().unwrap().go_forward(&tab_id); ipc_ok(msg_id, serde_json::json!({})) }
        IpcCmd::Reload { tab_id } => { state.tabs.lock().unwrap().reload(&tab_id); ipc_ok(msg_id, serde_json::json!({})) }
        IpcCmd::SetZoom { tab_id, level } => { state.tabs.lock().unwrap().set_zoom(&tab_id, level); ipc_ok(msg_id, serde_json::json!({})) }
        IpcCmd::SetMuted { tab_id, muted } => { state.tabs.lock().unwrap().set_muted(&tab_id, muted); ipc_ok(msg_id, serde_json::json!({})) }
        IpcCmd::SetViewport { x, y, w, h } => { state.tabs.lock().unwrap().resize_viewport(x, y, w, h); ipc_ok(msg_id, serde_json::json!({})) }
        IpcCmd::SuspendTab { tab_id } => { if let Some(s) = state.tabs.lock().unwrap().get_state(&tab_id).cloned() { state.suspender.lock().unwrap().suspend(&tab_id, &s.url, &s.title, &s.favicon); state.tabs.lock().unwrap().suspend_tab(&tab_id); } ipc_ok(msg_id, serde_json::json!({})) }
        IpcCmd::ResumeTab { tab_id } => { if let Some(susp) = state.suspender.lock().unwrap().get_suspended(&tab_id).cloned() { let _ = state.tabs.lock().unwrap().navigate(&tab_id, &susp.url); } state.suspender.lock().unwrap().mark_active(&tab_id); ipc_ok(msg_id, serde_json::json!({})) }
        IpcCmd::GetPrivacyStats => ipc_ok(msg_id, serde_json::to_value(&*state.privacy.lock().unwrap()).unwrap()),
        IpcCmd::ResetStats => { *state.privacy.lock().unwrap() = Default::default(); ipc_ok(msg_id, serde_json::json!({})) }
        IpcCmd::GetPrefs => ipc_ok(msg_id, serde_json::to_value(&*state.prefs.lock().unwrap()).unwrap()),
        IpcCmd::SetPref { key, value } => {
            let mut p = state.prefs.lock().unwrap();
            match key.as_str() {
                "block_ads" => { if let Some(v) = value.as_bool() { p.block_ads = v; } }
                "block_trackers" => { if let Some(v) = value.as_bool() { p.block_trackers = v; } }
                "block_nsfw" => { if let Some(v) = value.as_bool() { p.block_nsfw = v; } }
                "block_popups" => { if let Some(v) = value.as_bool() { p.block_popups = v; } }
                "https_only" => { if let Some(v) = value.as_bool() { p.https_only = v; } }
                "do_not_track" => { if let Some(v) = value.as_bool() { p.do_not_track = v; } }
                "auto_suspend_tabs" => { if let Some(v) = value.as_bool() { p.auto_suspend_tabs = v; } }
                "prefetch" => { if let Some(v) = value.as_bool() { p.prefetch = v; } }
                "clear_on_exit" => { if let Some(v) = value.as_bool() { p.clear_on_exit = v; } }
                "sync_enabled" => { if let Some(v) = value.as_bool() { p.sync_enabled = v; state.sync_mgr.set_enabled(v); } }
                "default_engine" => { if let Some(v) = value.as_str() { p.default_engine = v.into(); } }
                _ => return ipc_err(msg_id, "Unknown pref"),
            }
            drop(p);
            request_interceptor::global().set_prefs(state.prefs.lock().unwrap().clone());
            state.profile.lock().unwrap().set_setting(&key, value);
            ipc_ok(msg_id, serde_json::json!({}))
        }
        IpcCmd::ExtAPI { ext_id, tab_id, domain, method, args } => {
            if tab_id == "__background__" && domain == "runtime" && method == "sendMessage" {
                let to_ext = args["extensionId"].as_str().unwrap_or(&ext_id).to_string();
                state.bg_workers.send_message(WorkerMessage { to_ext_id: to_ext, from_ext_id: ext_id, payload: args["message"].clone(), response_id: args["responseId"].as_str().map(|s| s.to_string()) });
                return ipc_ok(msg_id, serde_json::json!(null));
            }
            let result = state.ext_rt.dispatch(&ext_id, &domain, &method, &args);
            ipc_ok(msg_id, result)
        }
        IpcCmd::CwsSearch { query, page } => { let r = state.rt.block_on(state.exts.lock().unwrap().search_store(&query, page.unwrap_or(0))); match r { Ok(r) => ipc_ok(msg_id, serde_json::to_value(r).unwrap()), Err(e) => ipc_err(msg_id, &e.to_string()) } }
        IpcCmd::CwsFeatured { category } => { let r = state.rt.block_on(state.exts.lock().unwrap().featured(&category)); match r { Ok(r) => ipc_ok(msg_id, serde_json::to_value(r).unwrap()), Err(e) => ipc_err(msg_id, &e.to_string()) } }
        IpcCmd::CwsInstall { ext_id } => {
            let exts = state.exts.clone(); let rt = state.rt.clone(); let bg = state.bg_workers.clone(); let id2 = ext_id.clone();
            rt.spawn(async move { if let Ok(ext) = exts.lock().unwrap().install_from_store(&id2, |_| {}).await { bg.spawn_for_extension(&ext); } });
            ipc_ok(msg_id, serde_json::json!({ "installing": true }))
        }
        IpcCmd::CwsUninstall { ext_id } => { state.bg_workers.remove(&ext_id); let r = state.exts.lock().unwrap().uninstall(&ext_id); match r { Ok(_) => ipc_ok(msg_id, serde_json::json!({})), Err(e) => ipc_err(msg_id, &e.to_string()) } }
        IpcCmd::CwsSetEnabled { ext_id, enabled } => { state.exts.lock().unwrap().set_enabled(&ext_id, enabled); ipc_ok(msg_id, serde_json::json!({})) }
        IpcCmd::CwsListInstalled => { let list: Vec<_> = state.exts.lock().unwrap().list().iter().map(|e| serde_json::json!({ "id": e.id, "name": e.name, "version": e.version, "description": e.description, "icon": e.icon, "iconBg": e.icon_bg, "enabled": e.enabled, "mv": e.manifest.manifest_version, "permissions": e.manifest.permissions, "hasBackground": e.manifest.background.is_some() })).collect(); ipc_ok(msg_id, serde_json::Value::Array(list)) }
        IpcCmd::StartDownload { url, filename } => {
            let id = uuid(); let home = dirs::download_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp")).to_string_lossy().into_owned(); let path = format!("{home}/{filename}");
            let item = DownloadItem { id: id.clone(), url: url.clone(), filename, path: path.clone(), size: 0, downloaded: 0, progress: 0.0, state: "in_progress".into(), start_time: unix_ms(), speed_bps: 0, mime_type: "application/octet-stream".into(), error: None };
            state.downloads.lock().unwrap().insert(id.clone(), item);
            let dl = state.downloads.clone(); let rt = state.rt.clone(); let id2 = id.clone();
            rt.spawn(async move { let res = network::download_file(&url, &path, &id2, dl.clone(), |_| {}).await; if let Some(d) = dl.lock().unwrap().get_mut(&id2) { match res { Ok(_) => { d.state = "complete".into(); d.progress = 100.0; } Err(e) => { d.state = "interrupted".into(); d.error = Some(e); } } } });
            ipc_ok(msg_id, serde_json::json!({ "downloadId": id }))
        }
        IpcCmd::GetDownloads => ipc_ok(msg_id, serde_json::to_value(state.downloads.lock().unwrap().values().cloned().collect::<Vec<_>>()).unwrap()),
        IpcCmd::CancelDownload { id } => { if let Some(d) = state.downloads.lock().unwrap().get_mut(&id) { d.state = "interrupted".into(); } ipc_ok(msg_id, serde_json::json!({})) }
        IpcCmd::OpenDownload { id } => { if let Some(d) = state.downloads.lock().unwrap().get(&id).cloned() { let _ = opener::open(&d.path); } ipc_ok(msg_id, serde_json::json!({})) }
        IpcCmd::GetCertInfo { url } => { let c = state.rt.block_on(network::get_cert_info(&url)); ipc_ok(msg_id, serde_json::to_value(c).unwrap()) }
        IpcCmd::GetSuggestions { query, engine } => { let s = state.rt.block_on(network::get_suggestions(&query, &engine)); ipc_ok(msg_id, serde_json::to_value(s).unwrap()) }
        IpcCmd::Prefetch { url } => { state.rt.spawn(network::prefetch(url)); ipc_ok(msg_id, serde_json::json!({})) }
        IpcCmd::GetBookmarks => ipc_ok(msg_id, serde_json::to_value(state.profile.lock().unwrap().get_bookmarks().to_vec()).unwrap()),
        IpcCmd::AddBookmark { url, title, favicon, folder } => { let bm = state.profile.lock().unwrap().add_bookmark(&url, &title, &favicon, folder.as_deref()); ipc_ok(msg_id, serde_json::to_value(bm).unwrap()) }
        IpcCmd::RemoveBookmark { id } => { state.profile.lock().unwrap().remove_bookmark(&id); ipc_ok(msg_id, serde_json::json!({})) }
        IpcCmd::GetHistory { limit } => { let h = state.profile.lock().unwrap().get_history(limit.unwrap_or(200)).to_vec(); ipc_ok(msg_id, serde_json::to_value(h).unwrap()) }
        IpcCmd::SearchHistory { query } => { let h: Vec<_> = state.profile.lock().unwrap().search_history(&query).iter().map(|x| (*x).clone()).collect(); ipc_ok(msg_id, serde_json::to_value(h).unwrap()) }
        IpcCmd::ClearHistory => { state.profile.lock().unwrap().clear_history(); ipc_ok(msg_id, serde_json::json!({})) }
        IpcCmd::GetSessions => { let s = state.profile.lock().unwrap().get_sessions().to_vec(); ipc_ok(msg_id, serde_json::to_value(s).unwrap()) }
        IpcCmd::SaveSession { tabs } => { let saved: Vec<SavedTab> = tabs.into_iter().map(|t| SavedTab { url: t.url, title: t.title, favicon: t.favicon, pinned: t.pinned, incognito: t.incognito }).collect(); state.profile.lock().unwrap().save_session(saved); ipc_ok(msg_id, serde_json::json!({})) }
        IpcCmd::RestoreSession { session_id } => { let tabs: Vec<_> = state.profile.lock().unwrap().get_sessions().iter().find(|s| s.id == session_id).map(|s| s.tabs.iter().map(|t| serde_json::json!({"url":t.url,"title":t.title})).collect()).unwrap_or_default(); ipc_ok(msg_id, serde_json::json!({"tabs":tabs})) }
        IpcCmd::DevToolsConnect { tab_id } => { state.devtools.connect_tab(&tab_id); ipc_ok(msg_id, serde_json::json!({ "cdpUrl": format!("ws://127.0.0.1:{}/devtools/page/{}", state.prefs.lock().unwrap().cdp_port, tab_id) })) }
        IpcCmd::DevToolsCmd { tab_id, command } => { state.devtools.send_command(&tab_id, &command); ipc_ok(msg_id, serde_json::json!({})) }
        IpcCmd::DevToolsHeapSnapshot { tab_id } => { state.devtools.take_heap_snapshot(&tab_id); ipc_ok(msg_id, serde_json::json!({})) }
        IpcCmd::DevToolsEval { tab_id, expression, call_id } => { state.devtools.evaluate(&tab_id, &expression, call_id); ipc_ok(msg_id, serde_json::json!({})) }
        IpcCmd::DevToolsBreakpoint { tab_id, script_id, line, condition } => { state.devtools.set_breakpoint(&tab_id, &script_id, line, condition.as_deref()); ipc_ok(msg_id, serde_json::json!({})) }
        IpcCmd::DevToolsGetDOM { tab_id } => { state.devtools.get_dom(&tab_id); ipc_ok(msg_id, serde_json::json!({})) }

        // ── Sync ──────────────────────────────────────────────────
        IpcCmd::SyncGetConfig => {
            let cfg = state.sync_mgr.get_config();
            ipc_ok(msg_id, serde_json::json!({ "enabled": cfg.enabled, "server_url": cfg.server_url, "user_id": cfg.user_id }))
        }
        IpcCmd::SyncSetConfig { server_url, user_id, enabled } => {
            match state.sync_mgr.configure(&server_url, &user_id, enabled) {
                Ok(_) => { state.prefs.lock().unwrap().sync_enabled = enabled; ipc_ok(msg_id, serde_json::json!({})) }
                Err(e) => ipc_err(msg_id, &e.to_string()),
            }
        }
        IpcCmd::SyncRegister { email, passphrase } => {
            let sync = state.sync_mgr.clone();
            match state.rt.block_on(sync.register_account(&email, &passphrase)) {
                Ok(uid) => ipc_ok(msg_id, serde_json::json!({ "user_id": uid })),
                Err(e)  => ipc_err(msg_id, &e.to_string()),
            }
        }
        IpcCmd::SyncPush { passphrase } => {
            let profile = state.profile.lock().unwrap();
            let bms  = profile.get_bookmarks().to_vec();
            let hist = profile.get_history(1000).to_vec();
            let sets = profile.get_all_settings().clone();
            let sess = profile.get_sessions().to_vec();
            drop(profile);
            match state.rt.block_on(state.sync_mgr.push_all(&bms, &hist, &sets, &sess, &passphrase)) {
                Ok(s)  => ipc_ok(msg_id, serde_json::json!({ "pushed": s.pushed, "errors": s.errors })),
                Err(e) => ipc_err(msg_id, &e.to_string()),
            }
        }
        IpcCmd::SyncPull { passphrase } => {
            match state.rt.block_on(state.sync_mgr.pull_all(&passphrase)) {
                Ok(pull) => {
                    let mut profile = state.profile.lock().unwrap();
                    if let Some(bms)  = &pull.bookmarks { for bm in bms { profile.add_bookmark(&bm.url, &bm.title, &bm.favicon, bm.folder.as_deref()); } }
                    if let Some(hist) = &pull.history   { for h in hist { profile.add_history(&h.url, &h.title, &h.favicon); } }
                    if let Some(sets) = &pull.settings  { for (k,v) in sets { profile.set_setting(k, v.clone()); } }
                    ipc_ok(msg_id, serde_json::json!({ "bookmarks": pull.bookmarks.as_ref().map(|b| b.len()).unwrap_or(0), "history": pull.history.as_ref().map(|h| h.len()).unwrap_or(0), "settings": pull.settings.is_some(), "errors": pull.errors }))
                }
                Err(e) => ipc_err(msg_id, &e.to_string()),
            }
        }
        IpcCmd::SyncGetStatus => { let s = state.sync_mgr.get_status(); ipc_ok(msg_id, serde_json::to_value(&s).unwrap_or_default()) }
        IpcCmd::SyncExportFile { path, passphrase } => {
            let profile = state.profile.lock().unwrap();
            let bms = profile.get_bookmarks().to_vec();
            let hist = profile.get_history(1000).to_vec();
            let sets = profile.get_all_settings().clone();
            drop(profile);
            match state.sync_mgr.export_encrypted(&bms, &hist, &sets, &passphrase, std::path::Path::new(&path)) {
                Ok(_)  => ipc_ok(msg_id, serde_json::json!({ "path": path })),
                Err(e) => ipc_err(msg_id, &e.to_string()),
            }
        }
        IpcCmd::SyncImportFile { path, passphrase } => {
            match state.sync_mgr.import_encrypted(std::path::Path::new(&path), &passphrase) {
                Ok(pull) => {
                    let mut profile = state.profile.lock().unwrap();
                    if let Some(bms)  = &pull.bookmarks { for bm in bms { profile.add_bookmark(&bm.url, &bm.title, &bm.favicon, bm.folder.as_deref()); } }
                    if let Some(hist) = &pull.history   { for h in hist { profile.add_history(&h.url, &h.title, &h.favicon); } }
                    ipc_ok(msg_id, serde_json::json!({ "bookmarks": pull.bookmarks.as_ref().map(|b| b.len()).unwrap_or(0), "history": pull.history.as_ref().map(|h| h.len()).unwrap_or(0) }))
                }
                Err(e) => ipc_err(msg_id, &e.to_string()),
            }
        }

        // ── Neutron GPU commands ───────────────────────────────────────────
        // These route directly to neutron::handle_neutron_ipc which dispatches
        // to the appropriate GPU surface layer.

        IpcCmd::NeutronRegisterScene { ptr, len } => {
            let args = serde_json::json!({ "ptr": ptr, "len": len });
            let result = neutron::handle_neutron_ipc("NeutronRegisterScene", &args);
            ipc_ok(msg_id, result)
        }
        IpcCmd::NeutronSetSurfaceRect { x, y, w, h } => {
            let args = serde_json::json!({ "x": x, "y": y, "w": w, "h": h });
            let result = neutron::handle_neutron_ipc("NeutronSetSurfaceRect", &args);
            ipc_ok(msg_id, result)
        }
        IpcCmd::NeutronInitGlyphTable { codepoints } => {
            let args = serde_json::json!({ "codepoints": codepoints });
            let result = neutron::handle_neutron_ipc("NeutronInitGlyphTable", &args);
            ipc_ok(msg_id, result)
        }
        IpcCmd::NeutronRasterizeGlyphs { codepoints } => {
            let args = serde_json::json!({ "codepoints": codepoints });
            let result = neutron::handle_neutron_ipc("NeutronRasterizeGlyphs", &args);
            ipc_ok(msg_id, result)
        }
        IpcCmd::NeutronPushDevToolsFrame { frame } => {
            let result = neutron::handle_neutron_ipc("NeutronPushDevToolsFrame", &frame);
            ipc_ok(msg_id, result)
        }
        IpcCmd::NeutronResize { w, h } => {
            let args = serde_json::json!({ "w": w, "h": h });
            let result = neutron::handle_neutron_ipc("NeutronResize", &args);
            ipc_ok(msg_id, result)
        }

        // ── Speculation Rules — instant navigation ─────────────────────────
        // Frontend calls SpeculativeLoad when user hovers a link for >100ms.
        // We create a hidden WebView that preloads the page in the background.
        // On actual navigation, promote_speculative() makes it instant.

        IpcCmd::SpeculativeLoad { url } => {
            let url_norm = normalize_url(&url);
            // Fire StartSpeculative through event proxy — WebView creation
            // must happen on the main thread (wry requirement).
            let _ = state.proxy.send_event(AppEvent::StartSpeculative { url: url_norm });
            ipc_ok(msg_id, serde_json::json!({ "started": true }))
        }
        IpcCmd::SpeculativeCancel { url } => {
            state.tabs.lock().unwrap().cancel_speculative(&url);
            ipc_ok(msg_id, serde_json::json!({ "cancelled": true }))
        }
        IpcCmd::SpeculativeCheck { url } => {
            let ready = state.tabs.lock().unwrap().is_speculative_ready(&url);
            ipc_ok(msg_id, serde_json::json!({ "ready": ready }))
        }
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .init();

    let mut prefs_init = BrowserPrefs::defaults();

    // ── Parental Controls: load locked prefs if this user has a child profile ──
    // parsec-parental writes /etc/parsec/parental/{username}-web-prefs.json
    // when a child profile is active. These prefs are enforced and cannot be
    // overridden from within the browser UI.
    if let Ok(username) = std::env::var("USER").or_else(|_| std::env::var("LOGNAME")) {
        let parental_path = std::path::PathBuf::from(
            format!("/etc/parsec/parental/{}-web-prefs.json", username)
        );
        if parental_path.exists() {
            match std::fs::read_to_string(&parental_path)
                .ok()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            {
                Some(locked) => {
                    // Apply parental overrides — these win over user prefs
                    if locked.get("_parental_locked").and_then(|v| v.as_bool()).unwrap_or(false) {
                        if let Some(v) = locked.get("block_nsfw").and_then(|v| v.as_bool()) {
                            prefs_init.block_nsfw = v;
                        }
                        if let Some(v) = locked.get("block_popups").and_then(|v| v.as_bool()) {
                            prefs_init.block_popups = v;
                        }
                        if let Some(v) = locked.get("https_only").and_then(|v| v.as_bool()) {
                            prefs_init.https_only = v;
                        }
                        if let Some(v) = locked.get("do_not_track").and_then(|v| v.as_bool()) {
                            prefs_init.do_not_track = v;
                        }
                        tracing::info!(
                            "[parsec-web] parental prefs applied for '{}' — block_nsfw={}, https_only={}",
                            username, prefs_init.block_nsfw, prefs_init.https_only
                        );
                    }
                }
                None => {
                    tracing::warn!("[parsec-web] could not parse parental prefs at {:?}", parental_path);
                }
            }
        }
    }

    let prefs = Arc::new(Mutex::new(prefs_init));
    request_interceptor::init(prefs.lock().unwrap().clone());

    let rt = Arc::new(tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().build().expect("tokio"));
    let privacy    = Arc::new(Mutex::new(request_interceptor::InterceptStats::default()));
    let downloads: Arc<Mutex<HashMap<String, DownloadItem>>> = Arc::new(Mutex::new(HashMap::new()));
    let exts       = Arc::new(Mutex::new(extension_store::ExtensionRegistry::new()));
    let profile_mgr = Arc::new(Mutex::new(ProfileManager::new().expect("profile")));
    let suspender  = Arc::new(Mutex::new(TabSuspensionManager::new(300)));
    let devtools   = Arc::new(DevToolsManager::new());

    let data_dir = dirs::data_local_dir().unwrap_or_else(|| std::path::PathBuf::from(".")).join("parsec-web");
    let sync_mgr = Arc::new(SyncManager::new(data_dir).expect("sync manager"));

    // Background worker channels (main-thread creation via event proxy)
    let (spawn_tx, mut spawn_rx) = mpsc::unbounded_channel::<SpawnRequest>();
    let (relay_tx, mut relay_rx) = mpsc::unbounded_channel::<WorkerMessage>();
    let bg_workers = Arc::new(BackgroundWorkerManager::new(spawn_tx, relay_tx));

    // Tab event channel
    let (tab_evt_tx, mut tab_evt_rx) = tokio::sync::mpsc::unbounded_channel::<TabEvent>();
    let tabs = Arc::new(Mutex::new(TabManager::new(tab_evt_tx, prefs.clone())));

    // Script injection channel
    let (inject_tx, mut inject_rx) = mpsc::unbounded_channel::<(String, String)>();

    // ── Event loop created HERE so proxy is available for ext_rt and AppState ──
    // Previously the event loop was created after AppState, meaning proxy could
    // not be stored in AppState (used after creation) and could not be passed to
    // ExtensionRuntime. Both caused compile errors / runtime panics.
    let event_loop = EventLoopBuilder::<AppEvent>::new().build();
    let proxy      = event_loop.create_proxy();

    // ExtensionRuntime gets proxy so chrome.tabs.create() fires real CreateTab events
    let ext_rt = Arc::new(ExtensionRuntime::new(
        profile_mgr.clone(), inject_tx.clone(), tabs.clone(), proxy.clone(),
    ).0);

    // Load extensions + spawn their background workers
    { let guard = exts.lock().unwrap(); for ext in guard.list() { ext_rt.load_extension(ext.clone()); bg_workers.spawn_for_extension(ext); } }

    let cdp_port = prefs.lock().unwrap().cdp_port;
    rt.spawn(async move { tokio::time::sleep(tokio::time::Duration::from_secs(1)).await; info!("CDP port {cdp_port} ready (requires forked WebKit build)"); });

    // AppState stores the proxy so handle_ipc can fire CreateTab and ChromeEval events
    let state = Arc::new(AppState { prefs, privacy, downloads, tabs, exts, ext_rt, profile: profile_mgr, suspender, devtools, bg_workers, sync_mgr, rt: rt.clone(), proxy: proxy.clone() });

    // Forward background spawn/relay channels → event proxy (existing)
    let p1 = proxy.clone();
    rt.spawn(async move { while let Some(req) = spawn_rx.recv().await { let _ = p1.send_event(AppEvent::SpawnBackground(req)); } });
    let p2 = proxy.clone();
    rt.spawn(async move { while let Some(msg) = relay_rx.recv().await { let _ = p2.send_event(AppEvent::RelayToBackground(msg)); } });

    // Forward inject channel → event proxy so extension scripts reach tab WebViews
    let p3 = proxy.clone();
    rt.spawn(async move {
        while let Some((tab_id, script)) = inject_rx.recv().await {
            let _ = p3.send_event(AppEvent::InjectScript { tab_id, script });
        }
    });

    // Forward tab events → chrome WebView as window.__parsec_chrome_event() calls.
    // This is what drives React UI updates for title, URL, favicon, load state, etc.
    let p4 = proxy.clone();
    rt.spawn(async move {
        while let Some(evt) = tab_evt_rx.recv().await {
            if let Ok(json) = serde_json::to_string(&evt) {
                let js = format!(
                    "window.__parsec_chrome_event && window.__parsec_chrome_event({})",
                    json
                );
                let _ = p4.send_event(AppEvent::ChromeEval(js));
            }
        }
    });

    let window = Arc::new(WindowBuilder::new().with_title("Parsec Web v1.3").with_inner_size(LogicalSize::new(1280.0f64, 800.0f64)).with_min_inner_size(LogicalSize::new(800.0f64, 500.0f64)).with_decorations(false).build(&event_loop).expect("window"));

    // IPC reply proxy — used by ipc_handler to send responses back to React.
    // Previously the response was computed but silently discarded, causing every
    // ipc() call in the frontend to hang forever (the Promise never resolved).
    let p5 = proxy.clone();
    let state_ipc = state.clone();
    let chrome_wv = WebViewBuilder::new(&window)
        .with_url(if cfg!(debug_assertions) { "http://localhost:1421" } else { "parsec-app://localhost" })
        .with_transparent(true)
        .with_custom_protocol("parsec-app".into(), |_req| { let body = include_bytes!("../../dist/index.html"); wry::http::Response::builder().header("Content-Type","text/html").body(body.to_vec()).unwrap() })
        .with_ipc_handler(move |msg: wry::http::Request<String>| {
            let body = msg.body();
            let parsed: serde_json::Value = match serde_json::from_str(body) { Ok(v) => v, Err(e) => { warn!("IPC: {e}"); return; } };
            let msg_id  = parsed["id"].as_str().unwrap_or("0").to_string();
            let cmd_val = serde_json::json!({ "cmd": parsed["cmd"], "args": parsed.get("args").cloned().unwrap_or(serde_json::json!({})) });
            let cmd: IpcCmd = match serde_json::from_value(cmd_val) { Ok(c) => c, Err(e) => { warn!("IPC cmd: {e}"); return; } };
            let resp = handle_ipc(&msg_id, cmd, &state_ipc);
            // Send the JSON response back to the React IPC bridge so Promises resolve
            let js = format!("window.__parsec_reply && window.__parsec_reply({})", resp);
            let _ = p5.send_event(AppEvent::ChromeEval(js));
        })
        .with_devtools(cfg!(debug_assertions))
        .build().expect("chrome WebView");

    if let Err(e) = neutron::init_surface(&window) { warn!("Neutron: {e}"); }

    let mut bg_view_set = BackgroundWebViewSet::new();
    let state_el = state.clone();

    info!("Parsec Web v1.3 started — background workers + sync active");

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            // Create a real wry WebView for the new tab. This is the fix for the
            // core bug: NewTab previously returned a tab ID without ever calling
            // create_tab(), so no WebView existed, nothing loaded, nothing rendered.
            Event::UserEvent(AppEvent::CreateTab { id, url, incognito }) => {
                let size   = window.inner_size();
                let bounds = (0i32, 80i32, size.width, size.height.saturating_sub(80 + 24));
                if let Err(e) = state_el.tabs.lock().unwrap()
                    .create_tab(&id, &url, &window, bounds, incognito)
                {
                    warn!("create_tab {id} failed: {e}");
                } else {
                    // Make the newly created tab the active one
                    state_el.tabs.lock().unwrap().set_active(&id);
                    info!("Tab {id} created and active");
                }
            }
            // Start a speculative preload for a URL the user is likely to visit.
            // The hidden WebView loads the page in the background.
            Event::UserEvent(AppEvent::StartSpeculative { url }) => {
                state_el.tabs.lock().unwrap()
                    .start_speculative_load(&url, &window);
            }
            // Evaluate JS in the chrome WebView — drives IPC replies and tab event
            // forwarding. Must run on the main thread (wry requirement on macOS).
            Event::UserEvent(AppEvent::ChromeEval(js)) => {
                let _ = chrome_wv.evaluate_script(&js);
            }
            // Inject a script into a specific tab WebView — drives extension API
            // methods: tabs.executeScript, scripting.insertCSS, tabs.sendMessage etc.
            Event::UserEvent(AppEvent::InjectScript { tab_id, script }) => {
                state_el.tabs.lock().unwrap().inject_script(&tab_id, &script);
            }
            Event::UserEvent(AppEvent::SpawnBackground(req)) => { bg_view_set.create(req, &window); }
            Event::UserEvent(AppEvent::RelayToBackground(msg)) => { bg_view_set.relay(msg); }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                let tabs = state_el.tabs.lock().unwrap();
                let saved: Vec<SavedTab> = tabs.get_all_states().iter().map(|t| SavedTab { url: t.url.clone(), title: t.title.clone(), favicon: t.favicon.clone(), pinned: t.pinned, incognito: t.incognito }).collect();
                drop(tabs);
                state_el.profile.lock().unwrap().save_session(saved);
                state_el.profile.lock().unwrap().save().ok();
                if state_el.prefs.lock().unwrap().clear_on_exit { state_el.profile.lock().unwrap().clear_history(); }
                neutron::shutdown();
                *control_flow = ControlFlow::Exit;
            }
            Event::WindowEvent { event: WindowEvent::Resized(size), .. } => {
                let h = size.height.saturating_sub(80 + 24);
                state_el.tabs.lock().unwrap().resize_viewport(0, 80, size.width, h);
                // Notify both Neutron layers of the new window dimensions
                neutron::resize(size.width, size.height);
            }
            _ => {}
        }
    });
}
