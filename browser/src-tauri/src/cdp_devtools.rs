// src-tauri/src/cdp_devtools.rs
//
// v1.3: Chrome DevTools Protocol integration.
//
// Our WebKit patch (0002) embeds a CDP WebSocket server at
// ws://127.0.0.1:9222. This module:
//
//   1. Discovers CDP targets (one per tab) via GET /json
//   2. Connects to each tab's CDP WebSocket
//   3. Enables all CDP domains: Network, Debugger, DOM, Memory,
//      Performance, Console, Page, Runtime, CSS, Profiler
//   4. Proxies events to the React DevTools panel in the chrome
//   5. Proxies commands from the React panel back to WebKit
//
// The React DevTools panel is a full reimplementation of Chrome
// DevTools UI that runs inside our chrome WebView.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tracing::{info, warn, debug};
use anyhow::{Result, Context};
use futures_util::{SinkExt, StreamExt};

// ── CDP target ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CdpTarget {
    pub id:                    String,
    pub title:                 String,
    pub url:                   String,
    #[serde(rename = "webSocketDebuggerUrl")]
    pub web_socket_debugger_url: String,
}

// ── CDP session ────────────────────────────────────────────────────

pub struct CdpSession {
    pub target_id:  String,
    pub cmd_tx:     mpsc::UnboundedSender<String>,
    pub msg_count:  u64,
}

// ── Network request captured via CDP ─────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CdpNetworkRequest {
    pub request_id:  String,
    pub url:         String,
    pub method:      String,
    pub headers:     HashMap<String, String>,
    pub post_data:   Option<String>,
    pub timestamp:   f64,
    pub resource_type: String,
    pub initiator:   String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CdpNetworkResponse {
    pub request_id:   String,
    pub url:          String,
    pub status:       u32,
    pub status_text:  String,
    pub headers:      HashMap<String, String>,
    pub mime_type:    String,
    pub protocol:     String,    // "h3", "h2", "http/1.1"
    pub encoded_data_length: u64,
    pub timing:       NetworkTiming,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NetworkTiming {
    pub dns_start:       f64,
    pub dns_end:         f64,
    pub connect_start:   f64,
    pub connect_end:     f64,
    pub ssl_start:       f64,
    pub ssl_end:         f64,
    pub send_start:      f64,
    pub send_end:        f64,
    pub receive_headers_end: f64,
}

// ── Console message ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleMessage {
    pub source:     String,  // "javascript" | "network" | "console-api"
    pub level:      String,  // "log" | "warning" | "error" | "info" | "debug"
    pub text:       String,
    pub url:        Option<String>,
    pub line:       Option<u32>,
    pub column:     Option<u32>,
    pub stack_trace: Option<Value>,
    pub timestamp:  f64,
}

// ── JS call frame ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsCallFrame {
    pub function_name: String,
    pub script_id:     String,
    pub url:           String,
    pub line_number:   u32,
    pub column_number: u32,
}

// ── DevTools event sent to React panel ────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum DevToolsEvent {
    NetworkRequestWillBeSent  { request: CdpNetworkRequest },
    NetworkResponseReceived   { response: CdpNetworkResponse },
    NetworkLoadingFinished    { request_id: String, encoded_data_length: u64 },
    NetworkLoadingFailed      { request_id: String, error: String, cancelled: bool },
    ConsoleMessage            { message: ConsoleMessage },
    BreakpointHit             { call_frames: Vec<JsCallFrame>, reason: String },
    ScriptParsed              { script_id: String, url: String, start_line: u32 },
    HeapSnapshot              { chunks: Vec<String> },
    PerformancePaint          { name: String, timestamp: f64 },
    TargetInfo                { targets: Vec<CdpTarget> },
}

// ── CDP domain enablers ────────────────────────────────────────────

fn make_enable_commands() -> Vec<(u64, Value)> {
    let cmds = [
        ("Network.enable",     json!({ "maxPostDataSize": 65536, "maxResourceBufferSize": 10485760 })),
        ("Console.enable",     json!({})),
        ("Log.enable",         json!({})),
        ("Debugger.enable",    json!({ "maxScriptsCacheSize": 10000000 })),
        ("DOM.enable",         json!({})),
        ("CSS.enable",         json!({})),
        ("Page.enable",        json!({})),
        ("Runtime.enable",     json!({})),
        ("Performance.enable", json!({})),
        ("Profiler.enable",    json!({})),
        ("Memory.getDOMCounters", json!({})),
        ("Network.setCacheDisabled", json!({ "cacheDisabled": false })),
        ("Debugger.setAsyncCallStackDepth", json!({ "maxDepth": 32 })),
        ("Runtime.setCustomObjectFormatterEnabled", json!({ "enabled": true })),
    ];
    cmds.into_iter().enumerate().map(|(i, (method, params))| {
        (i as u64 + 1, json!({ "id": i+1, "method": method, "params": params }))
    }).collect()
}

// ── DevTools Manager ──────────────────────────────────────────────

pub struct DevToolsManager {
    sessions:   Arc<Mutex<HashMap<String, CdpSession>>>,
    event_tx:   mpsc::UnboundedSender<DevToolsEvent>,
    pub event_rx: Option<mpsc::UnboundedReceiver<DevToolsEvent>>,
    cdp_port:   u16,
}

impl DevToolsManager {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            sessions:  Arc::new(Mutex::new(HashMap::new())),
            event_tx:  tx,
            event_rx:  Some(rx),
            cdp_port:  9222,
        }
    }

    /// Start the CDP connection for a tab
    pub fn connect_tab(&self, tab_id: &str) {
        let tab_id   = tab_id.to_string();
        let port     = self.cdp_port;
        let sessions = self.sessions.clone();
        let event_tx = self.event_tx.clone();

        tokio::spawn(async move {
            if let Err(e) = Self::run_tab_session(tab_id, port, sessions, event_tx).await {
                warn!("CDP session error: {e}");
            }
        });
    }

    async fn run_tab_session(
        tab_id:   String,
        port:     u16,
        sessions: Arc<Mutex<HashMap<String, CdpSession>>>,
        event_tx: mpsc::UnboundedSender<DevToolsEvent>,
    ) -> Result<()> {
        // Wait for CDP server to be ready (WebKit initialises async)
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Discover target
        let targets_url = format!("http://127.0.0.1:{port}/json");
        let targets: Vec<CdpTarget> = reqwest::get(&targets_url).await
            .context("CDP /json")?
            .json().await
            .context("CDP targets parse")?;

        let target = targets.iter()
            .find(|t| t.id == tab_id)
            .ok_or_else(|| anyhow::anyhow!("Tab {tab_id} not in CDP targets"))?;

        info!("CDP: connecting to {}", target.web_socket_debugger_url);

        // Connect WebSocket
        use tokio_tungstenite::connect_async;
        let (ws, _) = connect_async(&target.web_socket_debugger_url).await
            .context("CDP WebSocket connect")?;
        let (mut ws_tx, mut ws_rx) = ws.split();

        // Enable all domains
        let enable_cmds = make_enable_commands();
        for (_, cmd) in &enable_cmds {
            let msg = tokio_tungstenite::tungstenite::Message::Text(cmd.to_string());
            ws_tx.send(msg).await.context("CDP enable send")?;
        }

        // Command channel — React panel sends CDP commands here
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<String>();
        {
            let mut s = sessions.lock().unwrap();
            s.insert(tab_id.clone(), CdpSession { target_id: tab_id.clone(), cmd_tx, msg_count: 1000 });
        }

        // Forward commands from React → WebKit
        let tab_id2 = tab_id.clone();
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                let msg = tokio_tungstenite::tungstenite::Message::Text(cmd);
                if ws_tx.send(msg).await.is_err() { break; }
            }
            info!("CDP command forwarder stopped for {tab_id2}");
        });

        // Process events from WebKit → React panel
        while let Some(msg) = ws_rx.next().await {
            let text = match msg {
                Ok(tokio_tungstenite::tungstenite::Message::Text(t)) => t,
                Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => break,
                _ => continue,
            };

            let json: Value = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(e) => { debug!("CDP JSON parse: {e}"); continue; }
            };

            // Route CDP events to DevTools panel
            if let Some(method) = json["method"].as_str() {
                if let Some(evt) = Self::translate_cdp_event(method, &json["params"]) {
                    let _ = event_tx.send(evt);
                }
            }
        }

        info!("CDP session ended for {tab_id}");
        sessions.lock().unwrap().remove(&tab_id);
        Ok(())
    }

    /// Translate raw CDP events into typed DevToolsEvents for the React panel
    fn translate_cdp_event(method: &str, params: &Value) -> Option<DevToolsEvent> {
        match method {
            "Network.requestWillBeSent" => {
                let req = &params["request"];
                Some(DevToolsEvent::NetworkRequestWillBeSent {
                    request: CdpNetworkRequest {
                        request_id:    params["requestId"].as_str()?.to_string(),
                        url:           req["url"].as_str()?.to_string(),
                        method:        req["method"].as_str().unwrap_or("GET").to_string(),
                        headers:       params["request"]["headers"].as_object()
                            .map(|o| o.iter().map(|(k,v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect())
                            .unwrap_or_default(),
                        post_data:     req["postData"].as_str().map(|s| s.to_string()),
                        timestamp:     params["timestamp"].as_f64().unwrap_or(0.0),
                        resource_type: params["type"].as_str().unwrap_or("Other").to_string(),
                        initiator:     params["initiator"]["type"].as_str().unwrap_or("other").to_string(),
                    }
                })
            }
            "Network.responseReceived" => {
                let resp = &params["response"];
                Some(DevToolsEvent::NetworkResponseReceived {
                    response: CdpNetworkResponse {
                        request_id:   params["requestId"].as_str()?.to_string(),
                        url:          resp["url"].as_str().unwrap_or("").to_string(),
                        status:       resp["status"].as_u64().unwrap_or(0) as u32,
                        status_text:  resp["statusText"].as_str().unwrap_or("").to_string(),
                        headers:      resp["headers"].as_object()
                            .map(|o| o.iter().map(|(k,v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect())
                            .unwrap_or_default(),
                        mime_type:    resp["mimeType"].as_str().unwrap_or("").to_string(),
                        protocol:     resp["protocol"].as_str().unwrap_or("").to_string(),
                        encoded_data_length: resp["encodedDataLength"].as_u64().unwrap_or(0),
                        timing:       serde_json::from_value(resp["timing"].clone()).unwrap_or_default(),
                    }
                })
            }
            "Network.loadingFinished" => Some(DevToolsEvent::NetworkLoadingFinished {
                request_id: params["requestId"].as_str()?.to_string(),
                encoded_data_length: params["encodedDataLength"].as_u64().unwrap_or(0),
            }),
            "Network.loadingFailed" => Some(DevToolsEvent::NetworkLoadingFailed {
                request_id: params["requestId"].as_str()?.to_string(),
                error:      params["errorText"].as_str().unwrap_or("").to_string(),
                cancelled:  params["canceled"].as_bool().unwrap_or(false),
            }),
            "Runtime.consoleAPICalled" | "Console.messageAdded" => {
                let (level, text, url, line) = if method == "Runtime.consoleAPICalled" {
                    let lvl = params["type"].as_str().unwrap_or("log").to_string();
                    let txt = params["args"].as_array()
                        .map(|a| a.iter().map(|v| v["value"].as_str().unwrap_or("").to_string()).collect::<Vec<_>>().join(" "))
                        .unwrap_or_default();
                    (lvl, txt, None, None)
                } else {
                    let msg = &params["message"];
                    (msg["level"].as_str().unwrap_or("log").to_string(),
                     msg["text"].as_str().unwrap_or("").to_string(),
                     msg["url"].as_str().map(|s| s.to_string()),
                     msg["line"].as_u64().map(|n| n as u32))
                };
                Some(DevToolsEvent::ConsoleMessage {
                    message: ConsoleMessage {
                        source:     "console-api".into(),
                        level, text, url, line, column: None, stack_trace: None,
                        timestamp: params["timestamp"].as_f64().unwrap_or(0.0),
                    }
                })
            }
            "Debugger.paused" => {
                let frames = params["callFrames"].as_array()
                    .map(|a| a.iter().filter_map(|f| {
                        let loc = &f["location"];
                        Some(JsCallFrame {
                            function_name: f["functionName"].as_str().unwrap_or("(anonymous)").to_string(),
                            script_id:     loc["scriptId"].as_str()?.to_string(),
                            url:           f["url"].as_str().unwrap_or("").to_string(),
                            line_number:   loc["lineNumber"].as_u64().unwrap_or(0) as u32,
                            column_number: loc["columnNumber"].as_u64().unwrap_or(0) as u32,
                        })
                    }).collect())
                    .unwrap_or_default();
                Some(DevToolsEvent::BreakpointHit {
                    call_frames: frames,
                    reason: params["reason"].as_str().unwrap_or("breakpoint").to_string(),
                })
            }
            "Debugger.scriptParsed" => Some(DevToolsEvent::ScriptParsed {
                script_id:  params["scriptId"].as_str()?.to_string(),
                url:        params["url"].as_str().unwrap_or("").to_string(),
                start_line: params["startLine"].as_u64().unwrap_or(0) as u32,
            }),
            _ => None,
        }
    }

    /// Send a CDP command from the React panel to a specific tab
    pub fn send_command(&self, tab_id: &str, command: &str) {
        let sessions = self.sessions.lock().unwrap();
        if let Some(session) = sessions.get(tab_id) {
            let _ = session.cmd_tx.send(command.to_string());
        }
    }

    /// Get heap snapshot for memory profiler
    pub fn take_heap_snapshot(&self, tab_id: &str) {
        let cmd = json!({
            "id": 9001,
            "method": "HeapProfiler.takeHeapSnapshot",
            "params": { "reportProgress": true, "captureNumericValue": true }
        });
        self.send_command(tab_id, &cmd.to_string());
    }

    /// Set a JavaScript breakpoint
    pub fn set_breakpoint(&self, tab_id: &str, script_id: &str, line: u32, condition: Option<&str>) {
        let cmd = json!({
            "id": 9002,
            "method": "Debugger.setBreakpoint",
            "params": {
                "location": { "scriptId": script_id, "lineNumber": line },
                "condition": condition.unwrap_or("")
            }
        });
        self.send_command(tab_id, &cmd.to_string());
    }

    /// Evaluate JS expression in page context
    pub fn evaluate(&self, tab_id: &str, expression: &str, call_id: u64) {
        let cmd = json!({
            "id": call_id,
            "method": "Runtime.evaluate",
            "params": {
                "expression": expression,
                "returnByValue": true,
                "awaitPromise": true,
                "includeCommandLineAPI": true
            }
        });
        self.send_command(tab_id, &cmd.to_string());
    }

    /// Get DOM tree
    pub fn get_dom(&self, tab_id: &str) {
        let cmd = json!({
            "id": 9003,
            "method": "DOM.getDocument",
            "params": { "depth": -1, "pierce": true }
        });
        self.send_command(tab_id, &cmd.to_string());
    }

    /// Disconnect from a tab
    pub fn disconnect_tab(&self, tab_id: &str) {
        self.sessions.lock().unwrap().remove(tab_id);
    }

    /// Update page title/URL info (called on navigation)
    pub fn update_page_info(&self, tab_id: &str, title: &str, url: &str) {
        debug!("DevTools: page info updated for {} — {} ({})", tab_id, title, url);
        // Forward to CDP server if WebKit is built with our patch
        // The CDP server maintains its own target registry; notify it here
        let cmd = serde_json::json!({
            "id": 0,
            "method": "Page.navigatedWithinDocument",
            "params": { "frameId": "main", "url": url }
        });
        self.send_command(tab_id, &cmd.to_string());
    }
}
