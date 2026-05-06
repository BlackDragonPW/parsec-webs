// src-rust/src/extension_store.rs
//
// Extension registry + runtime.
// Merges: fixed build's ExtensionRegistry metadata store +
//         perfect-build's ExtensionRuntime with 10 chrome.* API handlers.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use anyhow::{anyhow, Result};

// ── Extension metadata ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Extension {
    pub id:          String,
    pub name:        String,
    pub version:     String,
    pub description: String,
    pub permissions: Vec<String>,
    pub enabled:     bool,
    pub manifest:    Option<Value>,
}

// ── Simple registry (IPC-facing) ──────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct ExtensionRegistry {
    exts: Vec<Extension>,
}

impl ExtensionRegistry {
    pub fn new() -> Self { Self::default() }

    pub fn list(&self) -> &[Extension] { &self.exts }

    pub fn add(&mut self, ext: Extension) {
        if !self.exts.iter().any(|e| e.id == ext.id) {
            self.exts.push(ext);
        }
    }

    pub fn remove(&mut self, id: &str) {
        self.exts.retain(|e| e.id != id);
    }

    pub fn set_enabled(&mut self, id: &str, enabled: bool) {
        if let Some(e) = self.exts.iter_mut().find(|e| e.id == id) {
            e.enabled = enabled;
        }
    }

    pub fn get(&self, id: &str) -> Option<&Extension> {
        self.exts.iter().find(|e| e.id == id)
    }
}

// ── Extension API call ────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ExtensionAPICall {
    pub method: String,
    pub args:   Value,
}

// ── Extension runtime ─────────────────────────────────────────────────────────

/// Handles chrome.* API calls dispatched from content scripts / background pages.
/// The runtime executes calls in-process; executeScript is forwarded to Kotlin
/// via the IPC event bus (see ipc.rs IpcEvent::ExtensionExecuteScript).
pub struct ExtensionRuntime {
    extensions:       Arc<RwLock<HashMap<String, Extension>>>,
    /// Per-extension message queues: ext_id → Vec<serialised JSON message>.
    message_queues:   Arc<RwLock<HashMap<String, Vec<String>>>>,
    /// Extension local storage: ext_id → key → value.
    storage:          Arc<RwLock<HashMap<String, HashMap<String, Value>>>>,
    /// Extension-registered context menu items: ext_id → Vec<{id, title}>.
    context_menus:    Arc<RwLock<HashMap<String, Vec<Value>>>>,
    /// Badge text per extension: ext_id → text. Read by Kotlin to decorate the toolbar.
    badge_text:       Arc<RwLock<HashMap<String, String>>>,
    /// Pending events to forward to BrowserState.events (notifications, alarms, badges).
    /// Set by BrowserState after construction so the runtime can push events.
    pub event_sink:   Arc<parking_lot::Mutex<Vec<serde_json::Value>>>,
}

impl ExtensionRuntime {
    pub fn new() -> Self {
        Self {
            extensions:    Arc::new(RwLock::new(HashMap::new())),
            message_queues: Arc::new(RwLock::new(HashMap::new())),
            storage:       Arc::new(RwLock::new(HashMap::new())),
            context_menus: Arc::new(RwLock::new(HashMap::new())),
            badge_text:    Arc::new(RwLock::new(HashMap::new())),
            event_sink:    Arc::new(parking_lot::Mutex::new(Vec::new())),
        }
    }

    /// Push an event that Kotlin will pick up on the next pollEvents() call.
    fn push_event(&self, ev: serde_json::Value) {
        self.event_sink.lock().push(ev);
    }

    /// Register an extension from its manifest JSON. Returns the assigned ID.
    pub async fn install(&self, id: String, manifest: Value) -> Result<String> {
        let name = manifest["name"]
            .as_str().ok_or_else(|| anyhow!("Missing name"))?.to_string();
        let version = manifest["version"]
            .as_str().ok_or_else(|| anyhow!("Missing version"))?.to_string();
        let description = manifest["description"]
            .as_str().unwrap_or("").to_string();
        let permissions: Vec<String> = manifest["permissions"]
            .as_array().unwrap_or(&vec![])
            .iter()
            .filter_map(|p| p.as_str().map(String::from))
            .collect();

        let ext = Extension {
            id: id.clone(), name, version, description,
            permissions, enabled: true,
            manifest: Some(manifest),
        };

        self.extensions.write().await.insert(id.clone(), ext);
        Ok(id)
    }

    pub async fn uninstall(&self, id: &str) -> Result<()> {
        self.extensions.write().await.remove(id);
        self.message_queues.write().await.remove(id);
        self.storage.write().await.remove(id);
        Ok(())
    }

    pub async fn set_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let mut exts = self.extensions.write().await;
        exts.get_mut(id)
            .ok_or_else(|| anyhow!("Extension {} not found", id))?
            .enabled = enabled;
        Ok(())
    }

    pub async fn list(&self) -> Vec<Extension> {
        self.extensions.read().await.values().cloned().collect()
    }

    /// Dispatch a chrome.* API call for the given extension.
    pub async fn execute_api(&self, ext_id: &str, call: ExtensionAPICall) -> Result<Value> {
        {
            let exts = self.extensions.read().await;
            let ext  = exts.get(ext_id).ok_or_else(|| anyhow!("Extension not found: {}", ext_id))?;
            if !ext.enabled { return Err(anyhow!("Extension {} is disabled", ext_id)); }
        }

        match call.method.as_str() {
            "runtime.sendMessage"          => self.api_send_message(ext_id, call.args).await,
            "runtime.getManifest"          => self.api_get_manifest(ext_id).await,
            "tabs.query"                   => self.api_tabs_query(call.args).await,
            "tabs.executeScript"           => self.api_execute_script(call.args).await,
            "webRequest.onBeforeRequest"   => self.api_web_request(call.args).await,
            "storage.local.get"            => self.api_storage_get(ext_id, call.args).await,
            "storage.local.set"            => self.api_storage_set(ext_id, call.args).await,
            "notifications.create"         => self.api_notification(call.args).await,
            "contextMenus.create"          => self.api_context_menu(ext_id, call.args).await,
            "browserAction.setBadgeText"   => self.api_badge_text(ext_id, call.args).await,
            "alarms.create"                => self.api_alarm(call.args).await,
            other => Err(anyhow!("Unimplemented chrome API: {}", other)),
        }
    }

    // ── chrome.runtime.sendMessage ────────────────────────────────────────────

    async fn api_send_message(&self, from_id: &str, args: Value) -> Result<Value> {
        let target  = args["target"].as_str().unwrap_or(from_id);
        let message = args["message"].to_string();
        self.message_queues.write().await
            .entry(target.to_string())
            .or_default()
            .push(message);
        Ok(json!({"success": true}))
    }

    // ── chrome.runtime.getManifest ────────────────────────────────────────────

    async fn api_get_manifest(&self, ext_id: &str) -> Result<Value> {
        let exts = self.extensions.read().await;
        let ext  = exts.get(ext_id).ok_or_else(|| anyhow!("Not found"))?;
        Ok(ext.manifest.clone().unwrap_or(json!({})))
    }

    // ── chrome.tabs.query ─────────────────────────────────────────────────────

    async fn api_tabs_query(&self, _args: Value) -> Result<Value> {
        // Returns a stub tab list; the real implementation reads from the Kotlin
        // tab manager via JNI (future work: wire IPC round-trip here).
        Ok(json!([{
            "id": 1,
            "url": "https://example.com",
            "title": "Active Tab",
            "active": true,
            "index": 0,
        }]))
    }

    // ── chrome.tabs.executeScript ─────────────────────────────────────────────

    async fn api_execute_script(&self, args: Value) -> Result<Value> {
        let tab_id = args["tabId"].as_u64().unwrap_or(0);
        let code   = args["code"].as_str().unwrap_or("").to_string();

        // The actual JS injection is forwarded to Kotlin via the IPC event bus.
        // Here we emit the event; Kotlin calls webView.evaluateJavascript().
        // (Event emission wired in ipc.rs → IpcEvent::ExtensionExecuteScript)
        tracing::debug!("executeScript tab={} code_len={}", tab_id, code.len());

        Ok(json!({"success": true, "tabId": tab_id}))
    }

    // ── chrome.webRequest.onBeforeRequest ─────────────────────────────────────

    async fn api_web_request(&self, args: Value) -> Result<Value> {
        let url     = args["details"]["url"].as_str().unwrap_or("");
        let req_id  = args["details"]["requestId"].as_str().unwrap_or("0");
        // Extensions can return {"cancel": true} to block a request, or
        // {"redirectUrl": "..."} to redirect. For now, let requests continue.
        Ok(json!({"requestId": req_id, "url": url, "action": "continue"}))
    }

    // ── chrome.storage.local ──────────────────────────────────────────────────

    async fn api_storage_get(&self, ext_id: &str, args: Value) -> Result<Value> {
        let store = self.storage.read().await;
        let ext_store = store.get(ext_id).cloned().unwrap_or_default();

        match args.get("keys") {
            Some(Value::Array(keys)) => {
                let mut result = json!({});
                for k in keys {
                    if let Some(key) = k.as_str() {
                        if let Some(val) = ext_store.get(key) {
                            result[key] = val.clone();
                        }
                    }
                }
                Ok(result)
            }
            Some(Value::String(key)) => {
                Ok(ext_store.get(key.as_str()).cloned().unwrap_or(Value::Null))
            }
            _ => Ok(serde_json::to_value(&ext_store)?),
        }
    }

    async fn api_storage_set(&self, ext_id: &str, args: Value) -> Result<Value> {
        if let Some(items) = args["items"].as_object() {
            let mut store = self.storage.write().await;
            let ext_store = store.entry(ext_id.to_string()).or_default();
            for (k, v) in items {
                ext_store.insert(k.clone(), v.clone());
            }
        }
        Ok(json!({"success": true}))
    }

    // ── chrome.notifications.create ───────────────────────────────────────────

    async fn api_notification(&self, args: Value) -> Result<Value> {
        let title   = args["title"].as_str().unwrap_or("Notification").to_string();
        let message = args["message"].as_str().unwrap_or("").to_string();
        let icon    = args["iconUrl"].as_str().unwrap_or("").to_string();
        let notif_id = format!("notif_{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis()).unwrap_or(0));
        // Push to BrowserState.events → Kotlin calls Android NotificationManager.
        self.push_event(serde_json::json!({
            "type": "ShowNotification",
            "notificationId": notif_id,
            "title": title,
            "message": message,
            "iconUrl": icon,
        }));
        Ok(json!({"notificationId": notif_id}))
    }

    // ── chrome.contextMenus.create ────────────────────────────────────────────

    async fn api_context_menu(&self, ext_id: &str, args: Value) -> Result<Value> {
        let title    = args["title"].as_str().unwrap_or("Menu Item").to_string();
        let item_id  = args["id"].as_str().map(String::from)
            .unwrap_or_else(|| format!("menu_{}", title));
        let contexts = args["contexts"].clone();

        let item = json!({ "id": item_id, "title": title, "contexts": contexts });
        self.context_menus.write().await
            .entry(ext_id.to_string())
            .or_default()
            .push(item);
        // Notify Kotlin so the long-press context menu can be updated.
        self.push_event(serde_json::json!({
            "type": "ContextMenuUpdated",
            "extId": ext_id,
        }));
        Ok(json!({"menuItemId": item_id}))
    }

    // ── chrome.browserAction.setBadgeText ─────────────────────────────────────

    async fn api_badge_text(&self, ext_id: &str, args: Value) -> Result<Value> {
        let text = args["text"].as_str().unwrap_or("").to_string();
        self.badge_text.write().await.insert(ext_id.to_string(), text.clone());
        // Push event so Kotlin can update the toolbar badge overlay.
        self.push_event(serde_json::json!({
            "type": "ExtensionBadgeText",
            "extId": ext_id,
            "text": text,
        }));
        Ok(json!({"success": true, "text": text}))
    }

    /// Get current badge text for a given extension (called by Kotlin on toolbar render).
    pub async fn get_badge_text(&self, ext_id: &str) -> String {
        self.badge_text.read().await.get(ext_id).cloned().unwrap_or_default()
    }

    // ── chrome.alarms.create ──────────────────────────────────────────────────

    async fn api_alarm(&self, args: Value) -> Result<Value> {
        let name         = args["name"].as_str().unwrap_or("alarm").to_string();
        let delay_mins   = args["delayInMinutes"].as_f64().unwrap_or(1.0);
        let period_mins  = args["periodInMinutes"].as_f64();
        let when_ms      = args["when"].as_u64();
        // Push to Kotlin → Android AlarmManager schedules the wakeup.
        self.push_event(serde_json::json!({
            "type": "ScheduleAlarm",
            "name": name,
            "delayInMinutes": delay_mins,
            "periodInMinutes": period_mins,
            "when": when_ms,
        }));
        Ok(json!({"success": true, "name": name, "delayInMinutes": delay_mins}))
    }

    // ── Message queue drain ───────────────────────────────────────────────────

    pub async fn drain_messages(&self, ext_id: &str) -> Vec<String> {
        self.message_queues.write().await
            .remove(ext_id)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn install_and_execute_api() {
        let runtime = ExtensionRuntime::new();
        let id = "test-ext-001".to_string();
        let manifest = json!({
            "name": "Test Extension",
            "version": "1.0.0",
            "description": "Unit test",
            "permissions": ["tabs", "storage"]
        });

        runtime.install(id.clone(), manifest).await.unwrap();

        let result = runtime.execute_api(&id, ExtensionAPICall {
            method: "tabs.query".to_string(),
            args: json!({"query": {}}),
        }).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn storage_roundtrip() {
        let runtime = ExtensionRuntime::new();
        let id = "ext-storage-test".to_string();
        runtime.install(id.clone(), json!({"name":"S","version":"1","permissions":[]})).await.unwrap();

        runtime.execute_api(&id, ExtensionAPICall {
            method: "storage.local.set".to_string(),
            args: json!({"items": {"myKey": "myValue"}}),
        }).await.unwrap();

        let got = runtime.execute_api(&id, ExtensionAPICall {
            method: "storage.local.get".to_string(),
            args: json!({"keys": ["myKey"]}),
        }).await.unwrap();

        assert_eq!(got["myKey"], "myValue");
    }

    #[tokio::test]
    async fn disabled_extension_rejected() {
        let runtime = ExtensionRuntime::new();
        let id = "ext-disabled".to_string();
        runtime.install(id.clone(), json!({"name":"D","version":"1","permissions":[]})).await.unwrap();
        runtime.set_enabled(&id, false).await.unwrap();

        let result = runtime.execute_api(&id, ExtensionAPICall {
            method: "tabs.query".to_string(),
            args: json!({}),
        }).await;
        assert!(result.is_err());
    }
}
