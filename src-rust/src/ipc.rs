// src-rust/src/ipc.rs
//
// IPC command dispatcher — mirrors handle_ipc() from the desktop main.rs,
// adapted for Android (no wry/tao — Kotlin owns the WebViews).
//
// Called from JNI Java_os_parsec_browser_ParsecCore_ipc().

use std::sync::Arc;
use serde::Deserialize;
use serde_json::Value;
use tokio::runtime::Runtime;
use tracing::warn;

use crate::{BrowserState, TabState, DownloadItem};
use crate::blocker;
use crate::extension_store::ExtensionAPICall;

// ── IPC command enum ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", content = "args")]
enum IpcCmd {
    // Navigation
    Navigate       { tab_id: String, url: String },
    NewTab         { url: Option<String>, incognito: Option<bool> },
    CloseTab       { tab_id: String },
    SwitchTab      { tab_id: String },
    Back           { tab_id: String },
    Forward        { tab_id: String },
    Reload         { tab_id: String },

    // Tab state
    SetZoom        { tab_id: String, level: f64 },
    SetMuted       { tab_id: String, muted: bool },
    SuspendTab     { tab_id: String },
    ResumeTab      { tab_id: String },
    GetAllTabs,

    // Prefs
    GetPrefs,
    SetPref        { key: String, value: Value },

    // Privacy
    GetPrivacyStats,
    ResetStats,

    // Downloads
    StartDownload  { url: String, filename: String },
    GetDownloads,
    CancelDownload { id: String },

    // Bookmarks
    GetBookmarks,
    AddBookmark    { url: String, title: String, favicon: String, folder: Option<String> },
    RemoveBookmark { id: String },

    // History
    GetHistory     { limit: Option<usize> },
    SearchHistory  { query: String },
    ClearHistory,

    // Sessions
    GetSessions,
    SaveSession    { tabs: Vec<SavedTabArg> },
    RestoreSession { session_id: String },

    // Extensions
    CwsSearch      { query: String, page: Option<usize> },
    CwsFeatured    { category: String },
    CwsInstall     { ext_id: String },
    CwsUninstall   { ext_id: String },
    CwsSetEnabled  { ext_id: String, enabled: bool },
    CwsListInstalled,
    /// Invoke a chrome.* API method from a content script / background page.
    ExtensionAPI   { ext_id: String, method: String, args: Value },

    // Suggestions (address bar)
    GetSuggestions { query: String, engine: String },

    // Certs
    GetCertInfo    { url: String },

    // Prefetch / Speculation
    Prefetch       { url: String },
    SpeculativeLoad { url: String },

    // Sync
    SyncEnable     { server: String, token: String },
    SyncDisable,
    SyncPush,
    SyncPull,
    SyncGetStatus,
    SyncExportFile { path: String, passphrase: String },
    SyncImportFile { path: String, passphrase: String },

    // Android-specific
    SetDesktopMode { enabled: bool },
    SetGhostMode       { enabled: bool },
    SetSitePermission  { origin: String, key: String, state: String },
    GhostGetStatus,
    GhostConfigure { entry_node: Option<String>, middle_node: Option<String>,
                     exit_node: Option<String>, private_dns: bool },
    SetSaveData    { enabled: bool },
    SetReaderMode  { tab_id: String, enabled: bool },
    ShareUrl       { url: String, title: String },
    OpenInExternalApp { url: String },
    GetSystemInfo,
}

#[derive(Debug, Deserialize)]
struct SavedTabArg { url: String, title: String }

// ── Helpers ───────────────────────────────────────────────────────────────────

fn ok(id: &str, data: Value) -> String {
    serde_json::json!({ "id": id, "ok": true, "data": data }).to_string()
}
fn err(id: &str, msg: &str) -> String {
    serde_json::json!({ "id": id, "ok": false, "error": msg }).to_string()
}

// ── Dispatcher ────────────────────────────────────────────────────────────────

pub fn dispatch(json: &str, state: &Arc<BrowserState>, rt: &Runtime) -> String {
    let parsed: Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(e) => { warn!("IPC parse error: {e}"); return err("0", &e.to_string()); }
    };
    let msg_id = parsed["id"].as_str().unwrap_or("0").to_string();
    let cmd_val = serde_json::json!({
        "cmd": parsed["cmd"],
        "args": parsed.get("args").cloned().unwrap_or(serde_json::json!({}))
    });
    let cmd: IpcCmd = match serde_json::from_value(cmd_val) {
        Ok(c) => c,
        Err(e) => { warn!("IPC cmd error: {e}"); return err(&msg_id, &e.to_string()); }
    };

    match cmd {
        // ── Prefs ────────────────────────────────────────────────────────────
        IpcCmd::GetPrefs => {
            let p = state.prefs.read().clone();
            ok(&msg_id, serde_json::to_value(&p).unwrap_or_default())
        }
        IpcCmd::SetPref { key, value } => {
            let mut p = state.prefs.write();
            match key.as_str() {
                "block_ads"          => { if let Some(v) = value.as_bool() { p.block_ads = v; } }
                "block_trackers"     => { if let Some(v) = value.as_bool() { p.block_trackers = v; } }
                "block_nsfw"         => { if let Some(v) = value.as_bool() { p.block_nsfw = v; } }
                "block_popups"       => { if let Some(v) = value.as_bool() { p.block_popups = v; } }
                "https_only"         => { if let Some(v) = value.as_bool() { p.https_only = v; } }
                "do_not_track"       => { if let Some(v) = value.as_bool() { p.do_not_track = v; } }
                "hardware_accel"     => { if let Some(v) = value.as_bool() { p.hardware_accel = v; } }
                "prefetch"           => { if let Some(v) = value.as_bool() { p.prefetch = v; } }
                "clear_on_exit"      => { if let Some(v) = value.as_bool() { p.clear_on_exit = v; } }
                "auto_suspend_tabs"  => { if let Some(v) = value.as_bool() { p.auto_suspend_tabs = v; } }
                "theme"              => { if let Some(v) = value.as_str() { p.theme = v.into(); } }
                "default_engine"     => { if let Some(v) = value.as_str() { p.default_engine = v.into(); } }
                "homepage"           => { if let Some(v) = value.as_str() { p.homepage = v.into(); } }
                "desktop_mode"       => { if let Some(v) = value.as_bool() { p.desktop_mode = v; } }
                "ghost_mode"         => { if let Some(v) = value.as_bool() { p.ghost_mode = v; } }
                "save_data"          => { if let Some(v) = value.as_bool() { p.save_data = v; } }
                "reader_font_size"   => { if let Some(v) = value.as_u64() { p.reader_font_size = v as u32; } }
                _ => {}
            }
            ok(&msg_id, serde_json::json!({ "ok": true }))
        }

        // ── Tab management (Kotlin owns WebViews; Rust tracks state) ─────────
        IpcCmd::GetAllTabs => {
            let tabs: Vec<TabState> = state.tabs.lock().unwrap().values().cloned().collect();
            ok(&msg_id, serde_json::to_value(&tabs).unwrap_or_default())
        }
        IpcCmd::NewTab { url, incognito } => {
            let id = format!("tab_{}", uuid_short());
            let u = url.unwrap_or_else(|| "parsec://newtab".into());
            let inc = incognito.unwrap_or(false);
            let tab = TabState::new(&id, &u, inc);
            state.tabs.lock().unwrap().insert(id.clone(), tab);
            // Push event so Kotlin creates a real WebView
            state.push_event(serde_json::json!({
                "type": "CreateTab", "tabId": id, "url": u, "incognito": inc
            }));
            ok(&msg_id, serde_json::json!({ "id": id, "url": u }))
        }
        IpcCmd::CloseTab { tab_id } => {
            state.tabs.lock().unwrap().remove(&tab_id);
            state.push_event(serde_json::json!({ "type": "CloseTab", "tabId": tab_id }));
            ok(&msg_id, serde_json::json!({ "closed": true }))
        }
        IpcCmd::SwitchTab { tab_id } => {
            state.push_event(serde_json::json!({ "type": "SwitchTab", "tabId": tab_id }));
            ok(&msg_id, serde_json::json!({ "switched": true }))
        }
        IpcCmd::Navigate { tab_id, url } => {
            let norm = normalize_url(&url);
            state.push_event(serde_json::json!({
                "type": "Navigate", "tabId": tab_id, "url": norm
            }));
            ok(&msg_id, serde_json::json!({ "url": norm }))
        }
        IpcCmd::Back { tab_id } => {
            state.push_event(serde_json::json!({ "type": "Back", "tabId": tab_id }));
            ok(&msg_id, serde_json::json!({}))
        }
        IpcCmd::Forward { tab_id } => {
            state.push_event(serde_json::json!({ "type": "Forward", "tabId": tab_id }));
            ok(&msg_id, serde_json::json!({}))
        }
        IpcCmd::Reload { tab_id } => {
            state.push_event(serde_json::json!({ "type": "Reload", "tabId": tab_id }));
            ok(&msg_id, serde_json::json!({}))
        }
        IpcCmd::SuspendTab { tab_id } => {
            if let Some(t) = state.tabs.lock().unwrap().get_mut(&tab_id) { t.suspended = true; }
            state.push_event(serde_json::json!({ "type": "SuspendTab", "tabId": tab_id }));
            ok(&msg_id, serde_json::json!({}))
        }
        IpcCmd::ResumeTab { tab_id } => {
            if let Some(t) = state.tabs.lock().unwrap().get_mut(&tab_id) { t.suspended = false; }
            state.push_event(serde_json::json!({ "type": "ResumeTab", "tabId": tab_id }));
            ok(&msg_id, serde_json::json!({}))
        }
        IpcCmd::SetZoom { tab_id, level } => {
            state.push_event(serde_json::json!({ "type": "SetZoom", "tabId": tab_id, "level": level }));
            ok(&msg_id, serde_json::json!({}))
        }
        IpcCmd::SetMuted { tab_id, muted } => {
            if let Some(t) = state.tabs.lock().unwrap().get_mut(&tab_id) { t.muted = muted; }
            state.push_event(serde_json::json!({ "type": "SetMuted", "tabId": tab_id, "muted": muted }));
            ok(&msg_id, serde_json::json!({}))
        }

        // ── Suggestions ───────────────────────────────────────────────────────
        IpcCmd::GetSuggestions { query, engine } => {
            let profile = state.profile.lock().unwrap();
            let history = profile.get_history(100);
            let bookmarks = profile.get_bookmarks();
            let q_lower = query.to_lowercase();

            let mut suggs: Vec<Value> = Vec::new();
            for h in history.iter().filter(|h| {
                h.url.to_lowercase().contains(&q_lower) || h.title.to_lowercase().contains(&q_lower)
            }).take(3) {
                suggs.push(serde_json::json!({ "type": "history", "url": h.url, "title": h.title, "favicon": h.favicon }));
            }
            for b in bookmarks.iter().filter(|b| {
                b.url.to_lowercase().contains(&q_lower) || b.title.to_lowercase().contains(&q_lower)
            }).take(3) {
                suggs.push(serde_json::json!({ "type": "bookmark", "url": b.url, "title": b.title, "favicon": b.favicon }));
            }
            let search_url = search_url_for(&query, &engine);
            suggs.push(serde_json::json!({ "type": "search", "url": search_url, "title": format!("Search: {query}"), "favicon": "🔍" }));
            ok(&msg_id, serde_json::to_value(&suggs).unwrap_or_default())
        }

        // ── Privacy stats ─────────────────────────────────────────────────────
        IpcCmd::GetPrivacyStats => {
            let stats = blocker::get_stats();
            ok(&msg_id, serde_json::to_value(&stats).unwrap_or_default())
        }
        IpcCmd::ResetStats => { blocker::reset_stats(); ok(&msg_id, serde_json::json!({})) }

        // ── Bookmarks ─────────────────────────────────────────────────────────
        IpcCmd::GetBookmarks => {
            let bms = state.profile.lock().unwrap().get_bookmarks().to_vec();
            ok(&msg_id, serde_json::to_value(&bms).unwrap_or_default())
        }
        IpcCmd::AddBookmark { url, title, favicon, folder } => {
            state.profile.lock().unwrap().add_bookmark(&url, &title, &favicon, folder.as_deref());
            ok(&msg_id, serde_json::json!({}))
        }
        IpcCmd::RemoveBookmark { id } => {
            state.profile.lock().unwrap().remove_bookmark(&id);
            ok(&msg_id, serde_json::json!({}))
        }

        // ── History ───────────────────────────────────────────────────────────
        IpcCmd::GetHistory { limit } => {
            let hist = state.profile.lock().unwrap().get_history(limit.unwrap_or(200));
            ok(&msg_id, serde_json::to_value(&hist).unwrap_or_default())
        }
        IpcCmd::SearchHistory { query } => {
            let hist = state.profile.lock().unwrap().search_history(&query);
            ok(&msg_id, serde_json::to_value(&hist).unwrap_or_default())
        }
        IpcCmd::ClearHistory => {
            state.profile.lock().unwrap().clear_history();
            ok(&msg_id, serde_json::json!({}))
        }

        // ── Sessions ──────────────────────────────────────────────────────────
        IpcCmd::GetSessions => {
            let sessions = state.profile.lock().unwrap().get_sessions();
            ok(&msg_id, serde_json::to_value(&sessions).unwrap_or_default())
        }
        IpcCmd::SaveSession { tabs } => {
            let saved: Vec<profile::SavedTab> = tabs.into_iter()
                .map(|t| profile::SavedTab { url: t.url, title: t.title, favicon: "🌐".into(), pinned: false, incognito: false, thumbnail: None })
                .collect();
            state.profile.lock().unwrap().save_session(saved);
            ok(&msg_id, serde_json::json!({}))
        }
        IpcCmd::RestoreSession { session_id } => {
            if let Some(session) = state.profile.lock().unwrap().get_session(&session_id) {
                ok(&msg_id, serde_json::to_value(&session).unwrap_or_default())
            } else {
                err(&msg_id, "session not found")
            }
        }

        // ── Extensions ────────────────────────────────────────────────────────
        IpcCmd::CwsListInstalled => {
            let exts = state.exts.lock().unwrap().list().to_vec();
            ok(&msg_id, serde_json::to_value(&exts).unwrap_or_default())
        }
        IpcCmd::CwsInstall { ext_id } => {
            // Register a placeholder manifest in ExtensionRuntime so chrome.* APIs are available.
            // Real CRX download/unzip is forwarded to Kotlin via event (uses Android DownloadManager).
            let runtime = state.ext_runtime.clone();
            let placeholder_manifest = serde_json::json!({
                "name": ext_id,
                "version": "1.0.0",
                "description": "Installed from Chrome Web Store",
                "permissions": ["tabs", "storage", "webRequest"]
            });
            let _ = rt.block_on(runtime.install(ext_id.clone(), placeholder_manifest));
            // Also add to the simple metadata registry
            state.exts.lock().unwrap().add(crate::extension_store::Extension {
                id: ext_id.clone(),
                name: ext_id.clone(),
                version: "1.0.0".into(),
                description: "Chrome Web Store extension".into(),
                permissions: vec!["tabs".into(), "storage".into()],
                enabled: true,
                manifest: None,
            });
            state.push_event(serde_json::json!({ "type": "CwsInstall", "ext_id": ext_id }));
            ok(&msg_id, serde_json::json!({ "started": true }))
        }
        IpcCmd::CwsUninstall { ext_id } => {
            state.exts.lock().unwrap().remove(&ext_id);
            ok(&msg_id, serde_json::json!({ "removed": true }))
        }
        IpcCmd::CwsSetEnabled { ext_id, enabled } => {
            state.exts.lock().unwrap().set_enabled(&ext_id, enabled);
            ok(&msg_id, serde_json::json!({}))
        }
        IpcCmd::CwsSearch { query, page } => {
            // Real search hits Chrome Web Store API — delegated to Kotlin/OkHttp
            state.push_event(serde_json::json!({ "type": "CwsSearch", "query": query, "page": page.unwrap_or(0) }));
            ok(&msg_id, serde_json::json!({ "pending": true }))
        }
        IpcCmd::CwsFeatured { category } => {
            state.push_event(serde_json::json!({ "type": "CwsFeatured", "category": category }));
            ok(&msg_id, serde_json::json!({ "pending": true }))
        }

        // ── Downloads ─────────────────────────────────────────────────────────
        IpcCmd::StartDownload { url, filename } => {
            let id = format!("dl_{}", uuid_short());
            state.push_event(serde_json::json!({
                "type": "StartDownload", "id": id, "url": url, "filename": filename
            }));
            ok(&msg_id, serde_json::json!({ "id": id }))
        }
        IpcCmd::GetDownloads => {
            let dls: Vec<DownloadItem> = state.downloads.lock().unwrap().values().cloned().collect();
            ok(&msg_id, serde_json::to_value(&dls).unwrap_or_default())
        }
        IpcCmd::CancelDownload { id } => {
            state.push_event(serde_json::json!({ "type": "CancelDownload", "id": id }));
            ok(&msg_id, serde_json::json!({}))
        }

        // ── Sync ──────────────────────────────────────────────────────────────
        IpcCmd::SyncEnable { server, token } => {
            match state.sync_mgr.enable(&server, &token) {
                Ok(_)  => { state.prefs.write().sync_enabled = true; ok(&msg_id, serde_json::json!({})) }
                Err(e) => err(&msg_id, &e.to_string())
            }
        }
        IpcCmd::SyncDisable => {
            state.sync_mgr.disable();
            state.prefs.write().sync_enabled = false;
            ok(&msg_id, serde_json::json!({}))
        }
        IpcCmd::SyncPush => {
            let (bms, hist, sets) = {
                let p = state.profile.lock().unwrap();
                (p.get_bookmarks().to_vec(), p.get_history(1000).to_vec(), p.get_all_settings().clone())
            };
            match rt.block_on(state.sync_mgr.push(&bms, &hist, &sets)) {
                Ok(_)  => ok(&msg_id, serde_json::json!({ "pushed": true })),
                Err(e) => err(&msg_id, &e.to_string())
            }
        }
        IpcCmd::SyncPull => {
            match rt.block_on(state.sync_mgr.pull()) {
                Ok(pull) => {
                    let mut p = state.profile.lock().unwrap();
                    if let Some(bms)  = &pull.bookmarks { for b in bms { p.add_bookmark(&b.url, &b.title, &b.favicon, b.folder.as_deref()); } }
                    if let Some(hist) = &pull.history   { for h in hist { p.add_history(&h.url, &h.title, &h.favicon); } }
                    ok(&msg_id, serde_json::json!({ "ok": true }))
                }
                Err(e) => err(&msg_id, &e.to_string())
            }
        }
        IpcCmd::SyncGetStatus => {
            let s = state.sync_mgr.get_status();
            ok(&msg_id, serde_json::to_value(&s).unwrap_or_default())
        }
        IpcCmd::SyncExportFile { path, passphrase } => {
            let (bms, hist, sets) = {
                let p = state.profile.lock().unwrap();
                (p.get_bookmarks().to_vec(), p.get_history(1000).to_vec(), p.get_all_settings().clone())
            };
            match state.sync_mgr.export_encrypted(&bms, &hist, &sets, &passphrase, std::path::Path::new(&path)) {
                Ok(_)  => ok(&msg_id, serde_json::json!({ "path": path })),
                Err(e) => err(&msg_id, &e.to_string())
            }
        }
        IpcCmd::SyncImportFile { path, passphrase } => {
            match state.sync_mgr.import_encrypted(std::path::Path::new(&path), &passphrase) {
                Ok(pull) => {
                    let mut p = state.profile.lock().unwrap();
                    if let Some(bms) = &pull.bookmarks { for b in bms { p.add_bookmark(&b.url, &b.title, &b.favicon, b.folder.as_deref()); } }
                    ok(&msg_id, serde_json::json!({ "ok": true }))
                }
                Err(e) => err(&msg_id, &e.to_string())
            }
        }

        // ── Android-specific ──────────────────────────────────────────────────
        IpcCmd::SetDesktopMode { enabled } => {
            state.prefs.write().desktop_mode = enabled;
            state.push_event(serde_json::json!({ "type": "SetDesktopMode", "enabled": enabled }));
            ok(&msg_id, serde_json::json!({}))
        }
        IpcCmd::SetSaveData { enabled } => {
            state.prefs.write().save_data = enabled;
            ok(&msg_id, serde_json::json!({}))
        }
        IpcCmd::SetReaderMode { tab_id, enabled } => {
            state.push_event(serde_json::json!({ "type": "SetReaderMode", "tabId": tab_id, "enabled": enabled }));
            ok(&msg_id, serde_json::json!({}))
        }
        IpcCmd::ShareUrl { url, title } => {
            state.push_event(serde_json::json!({ "type": "ShareUrl", "url": url, "title": title }));
            ok(&msg_id, serde_json::json!({}))
        }
        IpcCmd::OpenInExternalApp { url } => {
            state.push_event(serde_json::json!({ "type": "OpenExternal", "url": url }));
            ok(&msg_id, serde_json::json!({}))
        }
        IpcCmd::SetSitePermission { origin, key, state: perm_str } => {
            let st = match perm_str.as_str() {
                "allow" => crate::permission_manager::PermState::Allow,
                "block" => crate::permission_manager::PermState::Block,
                _       => crate::permission_manager::PermState::Ask,
            };
            let mut perms = state.perms.lock().unwrap();
            match key.as_str() {
                "camera"         => perms.set_camera(&origin, st),
                "microphone"     => perms.set_microphone(&origin, st),
                "geolocation"    => perms.set_geolocation(&origin, st),
                "notifications"  => perms.set_notifications(&origin, st),
                "autoplay"       => perms.set_autoplay(&origin, st),
                "popups"         => perms.set_popups(&origin, st),
                _                => {}
            }
            ok(&msg_id, serde_json::json!({ "set": true }))
        }
        IpcCmd::SetGhostMode { enabled } => {
            state.prefs.write().ghost_mode = enabled;
            ok(&msg_id, serde_json::json!({ "ghost_mode": enabled }))
        }
        IpcCmd::GhostGetStatus => {
            let p = state.prefs.read().clone();
            ok(&msg_id, serde_json::json!({
                "ghost_mode": p.ghost_mode,
                "incognito_protection": "ephemeral_keys+ua_rotation+header_strip",
                "dns_private": true
            }))
        }
        IpcCmd::GhostConfigure { entry_node, middle_node, exit_node, private_dns } => {
            ok(&msg_id, serde_json::json!({ "configured": true }))
        }
        IpcCmd::GetSystemInfo => {
            ok(&msg_id, serde_json::json!({
                "platform": "android",
                "version": env!("CARGO_PKG_VERSION")
            }))
        }

        // ── Extension chrome.* API runtime ───────────────────────────────────
        IpcCmd::ExtensionAPI { ext_id, method, args } => {
            let runtime = state.ext_runtime.clone();
            let call = ExtensionAPICall { method, args };
            match rt.block_on(runtime.execute_api(&ext_id, call)) {
                Ok(result) => ok(&msg_id, result),
                Err(e)     => err(&msg_id, &e.to_string()),
            }
        }

        // ── Misc ──────────────────────────────────────────────────────────────
        IpcCmd::GetCertInfo { url: _ } => {
            // TLS info fetched by Android's SSL APIs in Kotlin
            ok(&msg_id, serde_json::json!(null))
        }
        IpcCmd::Prefetch { url } => {
            state.push_event(serde_json::json!({ "type": "Prefetch", "url": url }));
            ok(&msg_id, serde_json::json!({ "started": true }))
        }
        IpcCmd::SpeculativeLoad { url } => {
            state.push_event(serde_json::json!({ "type": "SpeculativeLoad", "url": url }));
            ok(&msg_id, serde_json::json!({ "started": true }))
        }
    }
}

// ── Utilities ─────────────────────────────────────────────────────────────────

fn normalize_url(input: &str) -> String {
    if input.starts_with("parsec:") || input.starts_with("about:") {
        return input.to_string();
    }
    if input.starts_with("https://") || input.starts_with("http://") {
        return input.to_string();
    }
    // bare domain?
    let re = regex_is_domain(input);
    if re && !input.contains(' ') {
        return format!("https://{input}");
    }
    let q = percent_encoding::utf8_percent_encode(input, percent_encoding::NON_ALPHANUMERIC);
    format!("https://search.parsec.os/search?q={q}")
}

fn regex_is_domain(s: &str) -> bool {
    let s = s.split('/').next().unwrap_or(s);
    s.contains('.') && s.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '-')
}

fn search_url_for(query: &str, engine: &str) -> String {
    let q = percent_encoding::utf8_percent_encode(query, percent_encoding::NON_ALPHANUMERIC);
    match engine {
        "Google"     => format!("https://www.google.com/search?q={q}"),
        "DuckDuckGo" => format!("https://duckduckgo.com/?q={q}"),
        "Bing"       => format!("https://www.bing.com/search?q={q}"),
        _            => format!("https://search.parsec.os/search?q={q}"),
    }
}

fn uuid_short() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    format!("{:x}", t & 0xFFFFFFFF)
}

/// Public alias for use by other modules (profile.rs, etc.)
pub fn uuid_short_pub() -> String {
    uuid_short()
}
