# Parsec Android — Merged Build Notes

## What was merged

This is the definitive merged build combining:
- `parsec-android-fixed/` — full project structure, Kotlin UI, IPC, build system
- `parsec-android-perfect-build/` — correct algorithm implementations

---

## Fixes applied (all P0/P1/P2 issues from the analysis)

### ✅ P0 — SDF Rasteriser (`src-rust/src/sdf_rasteriser.rs`)

**Problem:** `rasterize_glyph()` returned zeroed UV coordinates for every glyph.
The atlas was never written to. Neutron chrome text (URL bar, toolbar, tab titles) was invisible.

**Fix:** Full implementation using `ttf-parser`:
- `ContourCollector` implements `OutlineBuilder` — samples line/quad/cubic segments into a point cloud
- `generate_sdf()` generates a real R8 signed-distance-field bitmap per glyph using:
  - Winding-number ray-casting `point_in_glyph()` for inside/outside test
  - `distance_to_edge()` for minimum edge distance
  - Maps to `[0,255]` with 128 = exactly on the glyph edge
- `SdfAtlas::pack()` implements shelf-packing with row advancement — `cursor_x/cursor_y/row_h` are now correctly maintained
- `SdfAtlas::blit()` writes the SDF bitmap into the atlas `Vec<u8>`
- Returned `GlyphMetrics` has real `uv_x/uv_y/uv_w/uv_h` normalised to atlas dimensions

### ✅ P0 — MANAGE_MEDIA removed (`android/app/src/main/AndroidManifest.xml`)

**Problem:** `android.permission.MANAGE_MEDIA` was present with `tools:ignore="ProtectedPermissions"` — suppresses lint but does NOT remove the Play Store restriction. Would cause rejection.

**Fix:** Permission entry removed entirely. `WRITE_EXTERNAL_STORAGE` (maxSdkVersion=28) and `READ_EXTERNAL_STORAGE` (maxSdkVersion=32) cover all needed download functionality.

### ✅ P1 — Sync crypto + HTTP push/pull (`src-rust/src/sync.rs`)

**Problem:** `push()` returned `Ok(())` immediately. `pull()` returned `Ok(SyncPull::default())`. `export_encrypted()` wrote `b"{}"`.

**Fix:**
- `SyncKey::from_passphrase()` derives a 32-byte key using PBKDF2-HMAC-SHA256 (100k iterations)
- `SyncKey::encrypt()` / `decrypt()` use ChaCha20-Poly1305 (AEAD)
- `SyncManager::enable()` generates a random 16-byte salt + 12-byte nonce
- `push()` serialises bookmarks+history+settings → encrypts → POST `/api/sync`
- `pull()` GET `/api/sync` → decrypts → returns `SyncPull` with all data
- `export_encrypted()` / `import_encrypted()` read/write `EncryptedPayload` JSON (salt + nonce + ciphertext, all base64)

### ✅ P1 — DoH Resolver (`src-rust/src/network.rs`)

**Problem:** `query_doh()` returned `Ok(vec!["127.0.0.1"])` unconditionally. The fixed build's `network.rs` was 12 lines with only HTTPS upgrade logic.

**Fix:**
- `build_dns_query()` builds a real RFC 1035 DNS wireformat A-record query
- `query_doh()` POSTs `application/dns-message` to Google/Cloudflare/Quad9 and parses the binary DNS response
- `parse_dns_response()` walks the DNS wireformat response and extracts A records
- 5-minute TTL cache via `Arc<RwLock<HashMap<String, (String, u64)>>>`
- `try_https_upgrade()` preserved and extended with full RFC 1918 ranges

### ✅ P1 — ExtensionRuntime added (`src-rust/src/extension_store.rs`)

**Problem:** `extension_runtime.rs` from the perfect-build existed but was never added to `src-rust/src/`. The fixed build's `extension_store.rs` only had metadata CRUD.

**Fix:** `ExtensionRuntime` merged into `extension_store.rs` with all 10 chrome.* API handlers:
- `runtime.sendMessage` — per-extension message queues
- `runtime.getManifest` — returns stored manifest JSON
- `tabs.query` — returns tab list (stub; wires to Kotlin tab manager in production)
- `tabs.executeScript` — logs and emits IPC event for Kotlin to call `evaluateJavascript()`
- `webRequest.onBeforeRequest` — extension-driven request interception hook
- `storage.local.get` / `storage.local.set` — per-extension in-memory KV store
- `notifications.create` — logs notification (wires to Android NotificationManager via JNI)
- `contextMenus.create` — stores menu items per extension
- `browserAction.setBadgeText` — logs badge text (wires to Kotlin UI via JNI)
- `alarms.create` — logs alarm (wires to Android AlarmManager via JNI)

### ✅ P2 — Safe Browsing (`BrowserActivity.kt`)

**Problem:** `WebView.setSafeBrowsingEnabled()` was never called. Phishing/malware URL checking was absent despite being a one-line fix.

**Fix:** Added `wv.setSafeBrowsingEnabled(true)` in `buildWebView()` before setting `webViewClient`.

### ✅ P1 — HSTS Manager (`src-rust/src/network.rs`)

**Problem:** `HSTSManager` had 3 hardcoded domains. Not integrated into the fixed build.

**Fix:**
- 30+ high-traffic domains in preload list (google.com, github.com, cloudflare.com, stripe.com, etc.)
- Runtime HSTS header recording via `record_hsts(domain, max_age)`
- TTL-expiry check on runtime entries
- `NetworkClient::get()` checks HSTS before making the request

---

## Remaining work (P3 — polish, not blocking)

- **Tab switcher thumbnails** — `TabSwitcherBottomSheet` uses `simple_list_item_2`. Replace with custom layout + `WebView.capturePicture()` thumbnails.
- **Find-in-page** — Replace `AlertDialog` with an inline bottom toolbar for match count + prev/next.
- **Full HSTS preload list** — Import the complete Chromium preload list (~120k entries) from a bundled assets JSON.
- **Atlas eviction** — SDF atlas is never evicted; on very long browsing sessions it will fill (2048×512 = 1MB). Add LRU eviction when `pack()` returns `None`.
- **DoH SCT validation** — CT verification is a passthrough; wire conscrypt SCT checking for Parsec's Rust HTTP client.
- **Tab groups / tab search** — UI-only gaps; `TabEntry` data model already supports them.

---

## Build

```bash
# Rust JNI library
cd src-rust
cargo build --target aarch64-linux-android --release

# Android app
cd android
./gradlew assembleRelease
```

Requires: Android NDK r26+, Rust `aarch64-linux-android` target, `cargo-ndk`.
