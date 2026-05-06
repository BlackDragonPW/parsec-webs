// src-tauri/src/certs.rs
pub use crate::network::CertInfo;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieRecord {
    pub name:      String,
    pub value:     String,
    pub domain:    String,
    pub path:      String,
    pub secure:    bool,
    pub http_only: bool,
    pub expires:   Option<u64>,
    pub same_site: String,
}
