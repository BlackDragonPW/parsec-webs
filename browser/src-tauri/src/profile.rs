// src-tauri/src/profile.rs
//
// Profile system: bookmarks, history, settings, extension list — persisted
// to disk with optional end-to-end encrypted sync via a user-provided key.
//
// Storage layout:
//   ~/.local/share/parsec-web/
//     profiles/
//       default/
//         profile.json        — name, avatar, creation date
//         bookmarks.json
//         history.json        — last 10,000 entries
//         settings.json
//         extensions.json     — installed extension IDs + states
//         sessions.json       — tab session restore data

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use anyhow::{Context, Result};
use tracing::{info, warn};

// ── Types ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id:         String,
    pub name:       String,
    pub avatar:     String,    // emoji
    pub created_at: u64,
    pub last_used:  u64,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BookmarkItem {
    pub id:      String,
    pub url:     String,
    pub title:   String,
    pub favicon: String,
    pub folder:  Option<String>,
    pub added:   u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HistoryItem {
    pub id:         String,
    pub url:        String,
    pub title:      String,
    pub visit_time: u64,
    pub favicon:    String,
    pub visit_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TabSession {
    pub id:        String,
    pub tabs:      Vec<SavedTab>,
    pub saved_at:  u64,
    pub label:     String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SavedTab {
    pub url:      String,
    pub title:    String,
    pub favicon:  String,
    pub pinned:   bool,
    pub incognito: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProfileData {
    pub bookmarks: Vec<BookmarkItem>,
    pub history:   Vec<HistoryItem>,
    pub sessions:  Vec<TabSession>,
    pub settings:  HashMap<String, serde_json::Value>,
}

// ── ProfileManager ─────────────────────────────────────────────────────────────

pub struct ProfileManager {
    base_dir:        PathBuf,
    current_profile: String,
    data:            ProfileData,
    profiles:        Vec<Profile>,
}

impl ProfileManager {
    pub fn new() -> Result<Self> {
        let base_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("parsec-web");
        std::fs::create_dir_all(base_dir.join("profiles").join("default"))?;

        let mut mgr = Self {
            base_dir,
            current_profile: "default".into(),
            data: ProfileData::default(),
            profiles: vec![],
        };
        mgr.load()?;
        Ok(mgr)
    }

    fn profile_dir(&self) -> PathBuf {
        self.base_dir.join("profiles").join(&self.current_profile)
    }

    pub fn load(&mut self) -> Result<()> {
        let dir = self.profile_dir();

        // Load bookmarks
        self.data.bookmarks = Self::load_json(&dir.join("bookmarks.json"))
            .unwrap_or_default();

        // Load history
        self.data.history = Self::load_json(&dir.join("history.json"))
            .unwrap_or_default();

        // Load settings
        self.data.settings = Self::load_json(&dir.join("settings.json"))
            .unwrap_or_default();

        // Load sessions
        self.data.sessions = Self::load_json(&dir.join("sessions.json"))
            .unwrap_or_default();

        info!("Profile '{}' loaded: {} bookmarks, {} history items",
            self.current_profile,
            self.data.bookmarks.len(),
            self.data.history.len());
        Ok(())
    }

    pub fn save(&self) -> Result<()> {
        let dir = self.profile_dir();
        Self::save_json(&dir.join("bookmarks.json"), &self.data.bookmarks)?;
        Self::save_json(&dir.join("history.json"),   &self.data.history)?;
        Self::save_json(&dir.join("settings.json"),  &self.data.settings)?;
        Self::save_json(&dir.join("sessions.json"),  &self.data.sessions)?;
        Ok(())
    }

    // ── Bookmarks ──────────────────────────────────────────────────────────────

    pub fn add_bookmark(&mut self, url: &str, title: &str, favicon: &str, folder: Option<&str>) -> BookmarkItem {
        // Remove duplicate if exists
        self.data.bookmarks.retain(|b| b.url != url);

        let bm = BookmarkItem {
            id:      uuid(),
            url:     url.into(),
            title:   title.into(),
            favicon: favicon.into(),
            folder:  folder.map(|s| s.into()),
            added:   now_ms(),
        };
        self.data.bookmarks.push(bm.clone());
        self.save().ok();
        bm
    }

    pub fn remove_bookmark(&mut self, id: &str) {
        self.data.bookmarks.retain(|b| b.id != id);
        self.save().ok();
    }

    pub fn get_bookmarks(&self) -> &[BookmarkItem] { &self.data.bookmarks }

    pub fn is_bookmarked(&self, url: &str) -> bool {
        self.data.bookmarks.iter().any(|b| b.url == url)
    }

    // ── History ────────────────────────────────────────────────────────────────

    pub fn add_history(&mut self, url: &str, title: &str, favicon: &str) {
        // Increment visit count if exists
        if let Some(item) = self.data.history.iter_mut().find(|h| h.url == url) {
            item.title      = title.into();
            item.visit_time = now_ms();
            item.visit_count += 1;
            let item = item.clone();
            // Move to front
            let pos = self.data.history.iter().position(|h| h.url == url).unwrap();
            self.data.history.remove(pos);
            self.data.history.insert(0, item);
        } else {
            self.data.history.insert(0, HistoryItem {
                id: uuid(), url: url.into(), title: title.into(),
                visit_time: now_ms(), favicon: favicon.into(), visit_count: 1,
            });
        }
        // Cap at 10,000
        self.data.history.truncate(10_000);
        // Save every 10 visits to avoid constant I/O
        if self.data.history.len() % 10 == 0 { self.save().ok(); }
    }

    pub fn get_history(&self, limit: usize) -> &[HistoryItem] {
        let end = self.data.history.len().min(limit);
        &self.data.history[..end]
    }

    pub fn search_history(&self, query: &str) -> Vec<&HistoryItem> {
        let q = query.to_lowercase();
        self.data.history.iter()
            .filter(|h| h.url.contains(&q) || h.title.to_lowercase().contains(&q))
            .take(50)
            .collect()
    }

    pub fn clear_history(&mut self) {
        self.data.history.clear();
        self.save().ok();
    }

    // ── Sessions ───────────────────────────────────────────────────────────────

    pub fn save_session(&mut self, tabs: Vec<SavedTab>) {
        // Keep last 10 sessions
        let session = TabSession {
            id:       uuid(),
            tabs,
            saved_at: now_ms(),
            label:    format!("Session {}", chrono_label()),
        };
        self.data.sessions.insert(0, session);
        self.data.sessions.truncate(10);
        self.save().ok();
    }

    pub fn get_sessions(&self) -> &[TabSession] { &self.data.sessions }

    pub fn get_last_session(&self) -> Option<&TabSession> { self.data.sessions.first() }

    // ── Settings ───────────────────────────────────────────────────────────────

    pub fn get_setting<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.data.settings.get(key)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    pub fn set_setting(&mut self, key: &str, value: serde_json::Value) {
        self.data.settings.insert(key.to_string(), value);
        self.save().ok();
    }

    pub fn get_all_settings(&self) -> &HashMap<String, serde_json::Value> {
        &self.data.settings
    }

    // ── Helpers ────────────────────────────────────────────────────────────────

    fn load_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn save_json<T: Serialize>(path: &Path, data: &T) -> Result<()> {
        let text = serde_json::to_string(data)?;
        std::fs::write(path, text).context("save json")?;
        Ok(())
    }
}

// ── Tab suspension ──────────────────────────────────────────────────────────────
//
// "Tab suspension" — unload the WebView content of background tabs
// while keeping the tab entry visible in the chrome.
// Reduces RAM proportionally to number of background tabs.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuspendedTab {
    pub tab_id:    String,
    pub url:       String,
    pub title:     String,
    pub favicon:   String,
    pub scroll_y:  f64,
    pub form_data: HashMap<String, String>,
}

pub struct TabSuspensionManager {
    suspended: HashMap<String, SuspendedTab>,
    /// Auto-suspend background tabs after N seconds of inactivity
    auto_suspend_after_secs: u64,
    last_activity: HashMap<String, u64>,
}

impl TabSuspensionManager {
    pub fn new(auto_suspend_secs: u64) -> Self {
        Self {
            suspended: HashMap::new(),
            auto_suspend_after_secs: auto_suspend_secs,
            last_activity: HashMap::new(),
        }
    }

    pub fn mark_active(&mut self, tab_id: &str) {
        self.last_activity.insert(tab_id.to_string(), now_ms());
        // Resume if suspended
        self.suspended.remove(tab_id);
    }

    pub fn suspend(&mut self, tab_id: &str, url: &str, title: &str, favicon: &str) {
        self.suspended.insert(tab_id.to_string(), SuspendedTab {
            tab_id:    tab_id.into(),
            url:       url.into(),
            title:     title.into(),
            favicon:   favicon.into(),
            scroll_y:  0.0,
            form_data: HashMap::new(),
        });
        info!("Tab {} suspended ({})", tab_id, url);
    }

    pub fn is_suspended(&self, tab_id: &str) -> bool {
        self.suspended.contains_key(tab_id)
    }

    pub fn get_suspended(&self, tab_id: &str) -> Option<&SuspendedTab> {
        self.suspended.get(tab_id)
    }

    /// Return tab IDs that should be suspended (inactive for too long)
    pub fn get_tabs_to_suspend(&self, active_tab_id: &str) -> Vec<String> {
        let threshold = now_ms().saturating_sub(self.auto_suspend_after_secs * 1000);
        self.last_activity.iter()
            .filter(|(id, &last)| {
                *id != active_tab_id &&
                !self.suspended.contains_key(*id) &&
                last < threshold
            })
            .map(|(id, _)| id.clone())
            .collect()
    }
}

// ── Utilities ─────────────────────────────────────────────────────────────────

fn uuid() -> String {
    // getrandom is already a transitive dependency (used by chacha20poly1305 and sync.rs).
    // Using subsec_nanos() was wrong: add two bookmarks in the same second and they
    // get the same ID, so deletion removes the wrong entry silently.
    let mut b = [0u8; 16];
    getrandom::getrandom(&mut b).unwrap_or_else(|_| {
        // Fallback if getrandom fails (shouldn't happen on any supported platform):
        // mix timestamp bits across all 16 bytes so at least the IDs differ.
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for (i, byte) in b.iter_mut().enumerate() {
            *byte = ((t >> (i * 5)) ^ (t >> (i * 3))) as u8;
        }
    });
    // Set UUID v4 version and variant bits (RFC 4122)
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0],b[1],b[2],b[3], b[4],b[5], b[6],b[7], b[8],b[9],
        b[10],b[11],b[12],b[13],b[14],b[15]
    )
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default().as_millis() as u64
}

fn chrono_label() -> String {
    // Proper Gregorian calendar math — days/365 was wrong (ignores leap years),
    // (days%365)/30 produced month 13 in December and day 32 in some months.
    let secs  = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default().as_secs();
    let days  = (secs / 86400) as u32;
    // Rata Die algorithm: count from 1970-01-01
    let z     = days + 719468;
    let era   = z / 146097;
    let doe   = z % 146097;
    let yoe   = (doe - doe/1460 + doe/36524 - doe/146096) / 365;
    let y     = yoe + era * 400;
    let doy   = doe - (365*yoe + yoe/4 - yoe/100);
    let mp    = (5*doy + 2) / 153;
    let d     = doy - (153*mp + 2)/5 + 1;
    let m     = if mp < 10 { mp + 3 } else { mp - 9 };
    let y     = if m <= 2 { y + 1 } else { y };
    format!("{y}-{m:02}-{d:02}")
}
