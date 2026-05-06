// src-rust/src/profile.rs — Profile persistence for Android
// Ported from desktop profile.rs — same data model, Android file paths.

use std::collections::HashMap;
use std::path::PathBuf;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HistoryItem {
    pub id: String, pub url: String, pub title: String,
    pub favicon: String, pub visit_time: u64, pub visit_count: u32,
    /// Frecency score — visit_count weighted by recency (Chrome algorithm).
    /// Computed: visit_count * exp(-age_days / 10) * 100. Higher = more relevant.
    #[serde(default)]
    pub frecency: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BookmarkItem {
    pub id: String, pub url: String, pub title: String,
    pub favicon: String, pub folder: Option<String>,
    #[serde(default)]
    pub added_time: u64,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedTab {
    pub url: String, pub title: String, pub favicon: String,
    pub pinned: bool, pub incognito: bool,
    /// Base64-encoded PNG thumbnail captured before suspend.
    #[serde(default)]
    pub thumbnail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabSession {
    pub id: String, pub label: String, pub saved_at: u64,
    pub tabs: Vec<SavedTab>,
}

/// Per-origin permission storage — persisted across sessions.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SitePermissions {
    /// origin → "allow" | "deny"
    pub camera:        HashMap<String, String>,
    pub microphone:    HashMap<String, String>,
    pub geolocation:   HashMap<String, String>,
    pub notifications: HashMap<String, String>,
}

#[derive(Debug, Default)]
pub struct ProfileManager {
    data_dir:    PathBuf,
    history:     Vec<HistoryItem>,
    bookmarks:   Vec<BookmarkItem>,
    sessions:    Vec<TabSession>,
    settings:    HashMap<String, serde_json::Value>,
    pub permissions: SitePermissions,
}

impl ProfileManager {
    pub fn new_at(data_dir: PathBuf) -> Result<Self> {
        let mut mgr = Self { data_dir: data_dir.clone(), ..Default::default() };
        mgr.load()?;
        Ok(mgr)
    }

    pub fn load(&mut self) -> Result<()> {
        let p = self.data_dir.join("profile.json");
        if let Ok(s) = std::fs::read_to_string(&p) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                if let Some(h) = v.get("history").and_then(|x| serde_json::from_value(x.clone()).ok()) { self.history = h; }
                if let Some(b) = v.get("bookmarks").and_then(|x| serde_json::from_value(x.clone()).ok()) { self.bookmarks = b; }
                if let Some(s) = v.get("sessions").and_then(|x| serde_json::from_value(x.clone()).ok()) { self.sessions = s; }
                if let Some(st) = v.get("settings").and_then(|x| serde_json::from_value(x.clone()).ok()) { self.settings = st; }
                if let Some(p) = v.get("permissions").and_then(|x| serde_json::from_value(x.clone()).ok()) { self.permissions = p; }
            }
        }
        Ok(())
    }

    pub fn save(&self) -> Result<()> {
        std::fs::create_dir_all(&self.data_dir)?;
        let v = serde_json::json!({
            "history": self.history, "bookmarks": self.bookmarks,
            "sessions": self.sessions, "settings": self.settings,
            "permissions": self.permissions,
        });
        // Atomic write: write to .tmp then rename to avoid corruption on crash.
        let tmp = self.data_dir.join("profile.json.tmp");
        let dest = self.data_dir.join("profile.json");
        std::fs::write(&tmp, v.to_string())?;
        std::fs::rename(&tmp, &dest)?;
        Ok(())
    }

    pub fn save_prefs<T: Serialize>(&self, prefs: &T) -> Result<()> {
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::write(self.data_dir.join("prefs.json"), serde_json::to_string(prefs)?)?;
        Ok(())
    }

    pub fn load_prefs<T: for<'de> Deserialize<'de>>(&self) -> Result<T> {
        let s = std::fs::read_to_string(self.data_dir.join("prefs.json"))?;
        Ok(serde_json::from_str(&s)?)
    }

    pub fn add_history(&mut self, url: &str, title: &str, favicon: &str) {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
        if let Some(existing) = self.history.iter_mut().find(|h| h.url == url) {
            existing.visit_time = now;
            existing.visit_count += 1;
            // Recompute frecency on update.
            existing.frecency = existing.visit_count as f64 * 100.0; // recency=1.0 since just visited
            return;
        }
        self.history.insert(0, HistoryItem {
            id: crate::ipc::uuid_short_pub(), url: url.into(), title: title.into(),
            favicon: favicon.into(), visit_time: now, visit_count: 1, frecency: 100.0,
        });
        // Chrome default: 50,000 entries. Evict lowest-frecency entries when over limit.
        if self.history.len() > 50_000 {
            // Recompute all frecencies then drop the bottom 10%.
            let now_ms = now;
            for h in self.history.iter_mut() {
                let age_days = (now_ms.saturating_sub(h.visit_time)) as f64 / 86_400_000.0;
                h.frecency = h.visit_count as f64 * (-age_days / 10.0_f64).exp() * 100.0;
            }
            self.history.sort_by(|a, b| b.frecency.partial_cmp(&a.frecency).unwrap_or(std::cmp::Ordering::Equal));
            self.history.truncate(50_000);
        }
    }

    pub fn get_history(&self, limit: usize) -> &[HistoryItem] {
        let n = limit.min(self.history.len());
        &self.history[..n]
    }

    /// Search history and return results sorted by frecency (most relevant first).
    pub fn search_history(&self, query: &str) -> Vec<HistoryItem> {
        let q = query.to_lowercase();
        let mut results: Vec<HistoryItem> = self.history.iter()
            .filter(|h| h.url.to_lowercase().contains(&q) || h.title.to_lowercase().contains(&q))
            .cloned()
            .collect();
        results.sort_by(|a, b| b.frecency.partial_cmp(&a.frecency).unwrap_or(std::cmp::Ordering::Equal));
        results
    }

    pub fn clear_history(&mut self) { self.history.clear(); }

    pub fn add_bookmark(&mut self, url: &str, title: &str, favicon: &str, folder: Option<&str>) {
        if self.bookmarks.iter().any(|b| b.url == url) { return; }
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
        self.bookmarks.push(BookmarkItem {
            id: crate::ipc::uuid_short_pub(), url: url.into(), title: title.into(),
            favicon: favicon.into(), folder: folder.map(|s| s.into()),
            added_time: now, tags: vec![],
        });
    }

    pub fn remove_bookmark(&mut self, id: &str) { self.bookmarks.retain(|b| b.id != id); }
    pub fn get_bookmarks(&self) -> &[BookmarkItem] { &self.bookmarks }

    /// Get or set a per-origin permission. permission_type: "camera"|"microphone"|"geolocation"|"notifications"
    pub fn set_permission(&mut self, origin: &str, permission_type: &str, value: &str) {
        let store = match permission_type {
            "camera"        => &mut self.permissions.camera,
            "microphone"    => &mut self.permissions.microphone,
            "geolocation"   => &mut self.permissions.geolocation,
            "notifications" => &mut self.permissions.notifications,
            _ => return,
        };
        store.insert(origin.to_string(), value.to_string());
    }

    pub fn get_permission(&self, origin: &str, permission_type: &str) -> Option<&str> {
        let store = match permission_type {
            "camera"        => &self.permissions.camera,
            "microphone"    => &self.permissions.microphone,
            "geolocation"   => &self.permissions.geolocation,
            "notifications" => &self.permissions.notifications,
            _ => return None,
        };
        store.get(origin).map(|s| s.as_str())
    }

    pub fn save_session(&mut self, tabs: Vec<SavedTab>) {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
        let label = format!("{} tabs", tabs.len());
        self.sessions.insert(0, TabSession { id: crate::ipc::uuid_short_pub(), label, saved_at: now, tabs });
        if self.sessions.len() > 50 { self.sessions.truncate(50); }
    }

    pub fn get_sessions(&self) -> &[TabSession] { &self.sessions }
    pub fn get_session(&self, id: &str) -> Option<&TabSession> { self.sessions.iter().find(|s| s.id == id) }
    pub fn set_setting(&mut self, key: &str, value: serde_json::Value) { self.settings.insert(key.into(), value); }
    pub fn get_all_settings(&self) -> &HashMap<String, serde_json::Value> { &self.settings }
}
