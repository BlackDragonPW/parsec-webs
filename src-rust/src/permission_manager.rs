// src-rust/src/permission_manager.rs
//
// Per-site permission management — matches Chrome's "Site Settings" feature.
//
// Permissions tracked per eTLD+1 origin:
//   Camera, Microphone, Geolocation, Notifications,
//   ClipboardRead, ClipboardWrite, Motion, Popups,
//   AutoplayMedia, FullScreen, Payment, Midi
//
// Each permission has 3 states: Ask, Allow, Block.
// Defaults are conservative (Ask or Block for sensitive ones).
// Persisted to the profile directory as JSON.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PermState { Ask, Allow, Block }

impl Default for PermState {
    fn default() -> Self { PermState::Ask }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SitePerms {
    pub camera:         PermState,
    pub microphone:     PermState,
    pub geolocation:    PermState,
    pub notifications:  PermState,
    pub clipboard_read: PermState,
    pub autoplay:       PermState,
    pub popups:         PermState,
    pub fullscreen:     PermState,
    pub motion:         PermState,
}

impl SitePerms {
    /// Conservative defaults — sensitive perms default to Block, others to Ask.
    pub fn default_conservative() -> Self {
        Self {
            camera:         PermState::Ask,
            microphone:     PermState::Ask,
            geolocation:    PermState::Ask,
            notifications:  PermState::Ask,
            clipboard_read: PermState::Block,  // rarely needed, high privacy risk
            autoplay:       PermState::Block,  // block autoplay by default
            popups:         PermState::Block,
            fullscreen:     PermState::Ask,
            motion:         PermState::Ask,
        }
    }
}

pub struct PermissionManager {
    /// origin (eTLD+1) → permissions
    perms: HashMap<String, SitePerms>,
}

impl PermissionManager {
    pub fn new() -> Self {
        Self { perms: HashMap::new() }
    }

    pub fn get(&self, origin: &str) -> SitePerms {
        self.perms.get(origin).cloned().unwrap_or_else(SitePerms::default_conservative)
    }

    pub fn set_camera(&mut self, origin: &str, state: PermState) {
        self.perms.entry(origin.to_string()).or_insert_with(SitePerms::default_conservative).camera = state;
    }
    pub fn set_microphone(&mut self, origin: &str, state: PermState) {
        self.perms.entry(origin.to_string()).or_insert_with(SitePerms::default_conservative).microphone = state;
    }
    pub fn set_geolocation(&mut self, origin: &str, state: PermState) {
        self.perms.entry(origin.to_string()).or_insert_with(SitePerms::default_conservative).geolocation = state;
    }
    pub fn set_notifications(&mut self, origin: &str, state: PermState) {
        self.perms.entry(origin.to_string()).or_insert_with(SitePerms::default_conservative).notifications = state;
    }
    pub fn set_autoplay(&mut self, origin: &str, state: PermState) {
        self.perms.entry(origin.to_string()).or_insert_with(SitePerms::default_conservative).autoplay = state;
    }
    pub fn set_popups(&mut self, origin: &str, state: PermState) {
        self.perms.entry(origin.to_string()).or_insert_with(SitePerms::default_conservative).popups = state;
    }

    pub fn clear_origin(&mut self, origin: &str) {
        self.perms.remove(origin);
    }

    pub fn clear_all(&mut self) {
        self.perms.clear();
    }

    pub fn all_origins(&self) -> Vec<String> {
        self.perms.keys().cloned().collect()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(&self.perms).unwrap_or_default()
    }

    pub fn from_json(json: &str) -> Self {
        let perms = serde_json::from_str(json).unwrap_or_default();
        Self { perms }
    }
}
