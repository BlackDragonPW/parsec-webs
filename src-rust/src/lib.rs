// ================================================================
//  parsec-core/src/lib.rs
//
//  Parsec Browser Android — Rust core (JNI layer)
//
//  All heavy logic from the desktop main.rs / tab_manager.rs /
//  network.rs / blocker.rs / sync.rs is preserved here, wired to
//  Android via JNI rather than tao/wry.
//
//  Architecture on Android:
//    ┌─────────────────────────────────────────────────┐
//    │  Kotlin UI (BrowserActivity + WebView tabs)     │
//    │    ↕ JNI calls (parsec_ipc / parsec_event_poll) │
//    │  Rust core (this file)                          │
//    │    ── blocking ── network ── sync ── GPU        │
//    └─────────────────────────────────────────────────┘
//
//  The Kotlin side owns the Android WebView instances (one per tab).
//  Rust handles: ad/tracker blocking, HTTPS upgrades, request
//  interception callbacks, profile persistence, sync, GPU chrome
//  compositor, extension store, download management.
// ================================================================

#![allow(non_snake_case)]

mod blocker;
mod network;
mod profile;
mod sync;
mod extension_store;
mod sdf_rasteriser;
mod neutron_android;
mod ipc;
mod phantom;
mod smart_cache;
mod permission_manager;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use jni::JNIEnv;
use jni::objects::{JClass, JString};
use jni::sys::{jstring, jboolean, jint, jlong, JNI_TRUE, JNI_FALSE};
use once_cell::sync::OnceCell;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::runtime::Runtime;
use tracing::{info, warn};

// ── Global runtime ────────────────────────────────────────────────────────────

static RT: OnceCell<Runtime> = OnceCell::new();
static STATE: OnceCell<Arc<BrowserState>> = OnceCell::new();

fn rt() -> &'static Runtime {
    RT.get().expect("parsec_init not called")
}

fn state() -> &'static Arc<BrowserState> {
    STATE.get().expect("parsec_init not called")
}

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub sync_enabled:       bool,
    // Android-specific
    pub desktop_mode:       bool,
    pub ghost_mode:         bool,  // true = encrypted incognito routing
    pub save_data:          bool,
    pub reader_font_size:   u32,
}

impl Default for BrowserPrefs {
    fn default() -> Self {
        Self {
            theme: "dark".into(),
            default_engine: "Parsec Search".into(),
            homepage: "parsec://newtab".into(),
            block_ads: true,
            block_trackers: true,
            block_nsfw: false,
            block_popups: true,
            https_only: true,
            do_not_track: true,
            hardware_accel: true,
            prefetch: true,
            clear_on_exit: false,
            auto_suspend_tabs: true,
            suspend_after_secs: 300,
            sync_enabled: false,
            desktop_mode: false,
            ghost_mode:   false,
            save_data: false,
            reader_font_size: 16,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabState {
    pub id:           String,
    pub url:          String,
    pub title:        String,
    pub favicon:      String,
    pub loading:      bool,
    pub can_go_back:  bool,
    pub can_go_fwd:   bool,
    pub pinned:       bool,
    pub muted:        bool,
    pub incognito:    bool,
    pub blocked:      bool,
    pub block_reason: Option<String>,
    pub suspended:    bool,
}

impl TabState {
    pub fn new(id: &str, url: &str, incognito: bool) -> Self {
        Self {
            id: id.into(), url: url.into(), title: "Loading…".into(),
            favicon: "🌐".into(), loading: url != "parsec://newtab",
            can_go_back: false, can_go_fwd: false,
            pinned: false, muted: false, incognito,
            blocked: false, block_reason: None, suspended: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadItem {
    pub id:          String,
    pub url:         String,
    pub filename:    String,
    pub size:        u64,
    pub downloaded:  u64,
    pub progress:    f32,
    pub state:       String,   // "downloading" | "done" | "error" | "cancelled"
    pub speed_bps:   u64,
    pub mime_type:   String,
    pub error:       Option<String>,
}

// ── Browser state ─────────────────────────────────────────────────────────────

pub struct BrowserState {
    prefs:     RwLock<BrowserPrefs>,
    tabs:      Mutex<HashMap<String, TabState>>,
    downloads: Mutex<HashMap<String, DownloadItem>>,
    // Pending events for Kotlin to poll (avoids JNI callbacks from Rust threads)
    events:    Mutex<Vec<serde_json::Value>>,
    profile:   Mutex<profile::ProfileManager>,
    sync_mgr:  Arc<sync::SyncManager>,
    exts:      Mutex<extension_store::ExtensionRegistry>,
    /// Full chrome.* API runtime — handles executeScript, storage, webRequest, etc.
    ext_runtime: Arc<extension_store::ExtensionRuntime>,
    phantom:     Arc<phantom::PhantomRouter>,
    cache:       Arc<smart_cache::SmartCache>,
    perms:       std::sync::Mutex<permission_manager::PermissionManager>,
}

impl BrowserState {
    fn push_event(&self, ev: serde_json::Value) {
        self.events.lock().unwrap().push(ev);
    }

    fn drain_events(&self) -> Vec<serde_json::Value> {
        let mut lock = self.events.lock().unwrap();
        std::mem::take(&mut *lock)
    }
}

// ── JNI entry points ──────────────────────────────────────────────────────────

/// Called once from ParsecApplication.onCreate()
#[no_mangle]
pub extern "system" fn Java_os_parsec_browser_ParsecCore_init(
    mut env: JNIEnv,
    _class: JClass,
    data_dir_j: JString,
) {
    // Init logging → Android logcat
    // Init logging → Android logcat (tracing-android 0.2 uses layer(), not init())
    {
        use tracing_subscriber::prelude::*;
        let _ = tracing_subscriber::registry()
            .with(tracing_android::layer("ParsecCore").unwrap())
            .init();
    }

    let data_dir: String = env.get_string(&data_dir_j)
        .map(|s| s.into())
        .unwrap_or_else(|_| "/data/data/os.parsec.browser/files".into());

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("tokio runtime");

    let _ = RT.set(rt);

    let data_path = std::path::PathBuf::from(&data_dir);
    let sync_mgr = Arc::new(sync::SyncManager::new(data_path.clone())
        .unwrap_or_else(|_| sync::SyncManager::noop()));

    let profile = profile::ProfileManager::new_at(data_path)
        .unwrap_or_else(|_| profile::ProfileManager::default());

    let state = Arc::new(BrowserState {
        prefs:       RwLock::new(BrowserPrefs::default()),
        tabs:        Mutex::new(HashMap::new()),
        downloads:   Mutex::new(HashMap::new()),
        events:      Mutex::new(Vec::new()),
        profile:     Mutex::new(profile),
        sync_mgr,
        exts:        Mutex::new(extension_store::ExtensionRegistry::new()),
        ext_runtime: Arc::new(extension_store::ExtensionRuntime::new()),
        phantom:     Arc::new(phantom::PhantomRouter::new()),
        cache:       Arc::new(smart_cache::SmartCache::new()),
        perms:       std::sync::Mutex::new(permission_manager::PermissionManager::new()),
    });

    // Load saved prefs
    {
        if let Ok(saved) = state.profile.lock().unwrap().load_prefs::<BrowserPrefs>() {
            *state.prefs.write() = saved;
        }
    }

    // Init blocker (loads ad/tracker block-lists from assets bundled in the APK)
    blocker::init();

    let _ = STATE.set(state);
    info!("Parsec Core initialized — data_dir={data_dir}");
}

/// Main IPC dispatcher — mirrors handle_ipc() from desktop main.rs.
/// Kotlin calls this synchronously; returns JSON string.
#[no_mangle]
pub extern "system" fn Java_os_parsec_browser_ParsecCore_ipc<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    json_j: JString<'local>,
) -> jstring {
    let json: String = match env.get_string(&json_j) {
        Ok(s) => s.into(),
        Err(_) => return env.new_string("{}").unwrap().into_raw(),
    };

    let result = ipc::dispatch(&json, state(), rt());
    env.new_string(result).unwrap().into_raw()
}

/// Poll pending events (tab title/URL changes, load progress, etc.).
/// Returns a JSON array of events. Kotlin calls this on a 16ms timer.
#[no_mangle]
pub extern "system" fn Java_os_parsec_browser_ParsecCore_pollEvents<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jstring {
    // Drain extension runtime events (notifications, alarms, badges) into main queue.
    {
        let ext_events: Vec<serde_json::Value> = state().ext_runtime.event_sink.lock().drain(..).collect();
        if !ext_events.is_empty() {
            let mut ev = state().events.lock().unwrap();
            ev.extend(ext_events);
        }
    }
    let events = state().drain_events();
    let json = serde_json::to_string(&events).unwrap_or_else(|_| "[]".into());
    env.new_string(json).unwrap().into_raw()
}

/// Called by Kotlin when a WebView navigates to a new URL.
/// Rust checks https_only, block-lists, etc.
/// Returns JSON: { "allow": bool, "redirect_url": string|null, "reason": string|null }
#[no_mangle]
pub extern "system" fn Java_os_parsec_browser_ParsecCore_shouldAllowNavigation<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    tab_id_j: JString<'local>,
    url_j: JString<'local>,
) -> jstring {
    let tab_id: String = env.get_string(&tab_id_j).map(|s| s.into()).unwrap_or_default();
    let url: String = env.get_string(&url_j).map(|s| s.into()).unwrap_or_default();

    let prefs = state().prefs.read().clone();

    // HTTPS upgrade
    if prefs.https_only {
        if let Some(upgraded) = network::try_https_upgrade(&url) {
            let resp = serde_json::json!({
                "allow": false,
                "redirect_url": upgraded,
                "reason": "https_upgrade"
            });
            return env.new_string(resp.to_string()).unwrap().into_raw();
        }
    }

    // Ad / tracker / popup blocking
    let block = blocker::should_block(&url, &prefs);
    if let Some(reason) = block {
        // Emit blocked event
        let ev = serde_json::json!({
            "type": "Blocked",
            "tabId": tab_id,
            "reason": reason
        });
        state().push_event(ev);

        let resp = serde_json::json!({
            "allow": false,
            "redirect_url": null,
            "reason": reason
        });
        return env.new_string(resp.to_string()).unwrap().into_raw();
    }

    let resp = serde_json::json!({ "allow": true, "redirect_url": null, "reason": null });
    env.new_string(resp.to_string()).unwrap().into_raw()
}

/// Called by Kotlin when a WebView fires an HTTP resource request.
/// Returns JSON: { "block": bool, "reason": string|null }
/// This is the subresource blocker — catches ads loaded inside pages.
#[no_mangle]
pub extern "system" fn Java_os_parsec_browser_ParsecCore_shouldBlockResource<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    _tab_id_j: JString<'local>,
    url_j: JString<'local>,
    resource_type_j: JString<'local>,
) -> jstring {
    let url: String = env.get_string(&url_j).map(|s| s.into()).unwrap_or_default();
    let _rtype: String = env.get_string(&resource_type_j).map(|s| s.into()).unwrap_or_default();

    let prefs = state().prefs.read().clone();
    let block = blocker::should_block_resource(&url, &prefs);

    let resp = if let Some(reason) = block {
        serde_json::json!({ "block": true, "reason": reason })
    } else {
        serde_json::json!({ "block": false, "reason": null })
    };
    env.new_string(resp.to_string()).unwrap().into_raw()
}

/// Tab lifecycle: notify Rust when a tab's URL/title changes.
#[no_mangle]
pub extern "system" fn Java_os_parsec_browser_ParsecCore_onTabUpdated<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    tab_id_j: JString<'local>,
    url_j: JString<'local>,
    title_j: JString<'local>,
    can_back: jboolean,
    can_fwd: jboolean,
    loading: jboolean,
) {
    let tab_id: String = env.get_string(&tab_id_j).map(|s| s.into()).unwrap_or_default();
    let url: String = env.get_string(&url_j).map(|s| s.into()).unwrap_or_default();
    let title: String = env.get_string(&title_j).map(|s| s.into()).unwrap_or_default();

    let mut tabs = state().tabs.lock().unwrap();
    let tab = tabs.entry(tab_id.clone()).or_insert_with(|| TabState::new(&tab_id, &url, false));
    tab.url = url.clone();
    tab.title = title.clone();
    tab.can_go_back = can_back == JNI_TRUE;
    tab.can_go_fwd = can_fwd == JNI_TRUE;
    tab.loading = loading == JNI_TRUE;

    // Save history — loading == JNI_FALSE means page finished loading
    if loading == JNI_FALSE && !url.starts_with("parsec://") && !tab.incognito {
        drop(tabs);
        state().profile.lock().unwrap().add_history(&url, &title, "🌐");
    }
}

/// Called on tab favicon change
#[no_mangle]
pub extern "system" fn Java_os_parsec_browser_ParsecCore_onFaviconChanged<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    tab_id_j: JString<'local>,
    favicon_url_j: JString<'local>,
) {
    let tab_id: String = env.get_string(&tab_id_j).map(|s| s.into()).unwrap_or_default();
    let favicon_url: String = env.get_string(&favicon_url_j).map(|s| s.into()).unwrap_or_default();

    if let Some(tab) = state().tabs.lock().unwrap().get_mut(&tab_id) {
        tab.favicon = favicon_url;
    }
}

/// Get suggestions for the address bar (search + history + bookmarks)
#[no_mangle]
pub extern "system" fn Java_os_parsec_browser_ParsecCore_getSuggestions<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    query_j: JString<'local>,
) -> jstring {
    let query: String = env.get_string(&query_j).map(|s| s.into()).unwrap_or_default();

    let profile = state().profile.lock().unwrap();
    let history = profile.get_history(200);
    let bookmarks = profile.get_bookmarks();

    let mut suggestions: Vec<serde_json::Value> = Vec::new();
    let q_lower = query.to_lowercase();

    // History matches
    for h in history.iter().filter(|h| {
        h.url.to_lowercase().contains(&q_lower) || h.title.to_lowercase().contains(&q_lower)
    }).take(3) {
        suggestions.push(serde_json::json!({
            "type": "history", "url": h.url, "title": h.title, "favicon": h.favicon
        }));
    }

    // Bookmark matches
    for b in bookmarks.iter().filter(|b| {
        b.url.to_lowercase().contains(&q_lower) || b.title.to_lowercase().contains(&q_lower)
    }).take(3) {
        suggestions.push(serde_json::json!({
            "type": "bookmark", "url": b.url, "title": b.title, "favicon": b.favicon
        }));
    }

    // Search suggestion
    let engine = state().prefs.read().default_engine.clone();
    let search_url = match engine.as_str() {
        "Google"     => format!("https://www.google.com/search?q={}", percent_encoding::utf8_percent_encode(&query, percent_encoding::NON_ALPHANUMERIC)),
        "DuckDuckGo" => format!("https://duckduckgo.com/?q={}", percent_encoding::utf8_percent_encode(&query, percent_encoding::NON_ALPHANUMERIC)),
        "Bing"       => format!("https://www.bing.com/search?q={}", percent_encoding::utf8_percent_encode(&query, percent_encoding::NON_ALPHANUMERIC)),
        _            => format!("https://search.parsec.os/search?q={}", percent_encoding::utf8_percent_encode(&query, percent_encoding::NON_ALPHANUMERIC)),
    };
    suggestions.push(serde_json::json!({
        "type": "search", "url": search_url, "title": format!("Search for \"{}\"", query), "favicon": "🔍"
    }));

    let json = serde_json::to_string(&suggestions).unwrap_or_else(|_| "[]".into());
    env.new_string(json).unwrap().into_raw()
}

/// Neutron GPU: init the wgpu surface on the Android SurfaceView.
/// surface_ptr is the ANativeWindow* cast to jlong.
#[no_mangle]
pub extern "system" fn Java_os_parsec_browser_ParsecCore_neutronInit(
    _env: JNIEnv,
    _class: JClass,
    surface_ptr: jlong,
    width: jint,
    height: jint,
) -> jboolean {
    match neutron_android::init(surface_ptr as *mut std::ffi::c_void, width as u32, height as u32) {
        Ok(_)  => { info!("Neutron GPU init OK {}×{}", width, height); JNI_TRUE }
        Err(e) => { warn!("Neutron GPU init failed: {e}"); JNI_FALSE }
    }
}

/// Neutron GPU: render one frame
#[no_mangle]
pub extern "system" fn Java_os_parsec_browser_ParsecCore_neutronFrame(
    _env: JNIEnv,
    _class: JClass,
) {
    neutron_android::render_frame();
}

/// Neutron GPU: surface resized
#[no_mangle]
pub extern "system" fn Java_os_parsec_browser_ParsecCore_neutronResize(
    _env: JNIEnv,
    _class: JClass,
    width: jint,
    height: jint,
) {
    neutron_android::resize(width as u32, height as u32);
}

/// Called when the app is going to background — pause GPU rendering
#[no_mangle]
pub extern "system" fn Java_os_parsec_browser_ParsecCore_onPause(
    _env: JNIEnv,
    _class: JClass,
) {
    neutron_android::pause();
    // Persist prefs + profile
    let prefs = state().prefs.read().clone();
    let _ = state().profile.lock().unwrap().save_prefs(&prefs);
    let _ = state().profile.lock().unwrap().save();
}

/// Called when app returns to foreground
#[no_mangle]
pub extern "system" fn Java_os_parsec_browser_ParsecCore_onResume(
    _env: JNIEnv,
    _class: JClass,
) {
    neutron_android::resume();
}

/// Full shutdown — called from onDestroy
#[no_mangle]
pub extern "system" fn Java_os_parsec_browser_ParsecCore_shutdown(
    _env: JNIEnv,
    _class: JClass,
) {
    let prefs = state().prefs.read().clone();
    let _ = state().profile.lock().unwrap().save_prefs(&prefs);
    let _ = state().profile.lock().unwrap().save();
    neutron_android::shutdown();
    info!("Parsec Core shutdown");
}

// ── Ghost Mode JNI entry points ───────────────────────────────────────────────

/// Called when an incognito tab is created — generate ephemeral keys.
#[no_mangle]
pub extern "system" fn Java_os_parsec_browser_ParsecCore_ghostCreateSession<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    tab_id_j: JString<'local>,
) {
    let tab_id: String = env.get_string(&tab_id_j).map(|s| s.into()).unwrap_or_default();
    rt().block_on(state().phantom.create_session(&tab_id));
}

/// Called when an incognito tab is closed — zero keys immediately.
#[no_mangle]
pub extern "system" fn Java_os_parsec_browser_ParsecCore_ghostDestroySession<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    tab_id_j: JString<'local>,
) {
    let tab_id: String = env.get_string(&tab_id_j).map(|s| s.into()).unwrap_or_default();
    rt().block_on(state().phantom.destroy_session(&tab_id));
}

/// Get the randomised user-agent for a ghost tab.
#[no_mangle]
pub extern "system" fn Java_os_parsec_browser_ParsecCore_ghostGetUserAgent<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    tab_id_j: JString<'local>,
) -> jstring {
    let tab_id: String = env.get_string(&tab_id_j).map(|s| s.into()).unwrap_or_default();
    let ua = rt().block_on(state().phantom.get_user_agent(&tab_id))
        .unwrap_or_else(|| "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/120.0.0.0 Safari/537.36".into());
    env.new_string(ua).unwrap().into_raw()
}

/// Get Ghost Mode status as JSON.
#[no_mangle]
pub extern "system" fn Java_os_parsec_browser_ParsecCore_ghostGetStatus<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jstring {
    let config  = rt().block_on(state().phantom.get_config());
    let sessions = rt().block_on(async { state().phantom.sessions.read().await.len() });
    let status  = phantom::GhostStatus {
        enabled:          state().prefs.read().ghost_mode,
        session_count:    sessions,
        has_proxy_server: config.entry_node.is_some(),
        hop_count:        if config.middle_node.is_some() { 3 } else if config.entry_node.is_some() { 2 } else { 0 },
        dns_private:      config.private_dns,
    };
    let json = serde_json::to_string(&status).unwrap_or_else(|_| "{}".into());
    env.new_string(json).unwrap().into_raw()
}

/// Configure phantom proxy servers (entry/middle/exit node URLs).
#[no_mangle]
pub extern "system" fn Java_os_parsec_browser_ParsecCore_ghostConfigure<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    config_json_j: JString<'local>,
) {
    let json: String = env.get_string(&config_json_j).map(|s| s.into()).unwrap_or_default();
    if let Ok(config) = serde_json::from_str::<phantom::PhantomConfig>(&json) {
        rt().block_on(state().phantom.configure(config));
    }
}

