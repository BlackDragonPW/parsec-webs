# Parsec Browser

A privacy-first browser with a Rust core, native Android UI, and a Tauri-based desktop app.

## Repository Structure

```
parsec-android/
├── android/                     # Android app (Kotlin + native Rust JNI)
│   ├── app/
│   │   ├── src/main/
│   │   │   ├── java/os/parsec/browser/
│   │   │   │   ├── ParsecApplication.kt    # App entry point, loads Rust .so
│   │   │   │   ├── ParsecCore.kt           # JNI bridge object
│   │   │   │   ├── ui/
│   │   │   │   │   ├── BrowserActivity.kt  # Main UI: WebViews, toolbar, tabs
│   │   │   │   │   ├── BrowserPanelFragment.kt  # History / Bookmarks / Downloads / Settings panel
│   │   │   │   │   ├── TabSwitcherBottomSheet.kt
│   │   │   │   │   └── MenuBottomSheet.kt
│   │   │   │   ├── adapter/
│   │   │   │   │   └── SuggestionAdapter.kt   # Address bar autocomplete
│   │   │   │   └── service/
│   │   │   │       └── DownloadService.kt     # Foreground download service
│   │   │   ├── res/                         # Layouts, drawables, strings, themes
│   │   │   └── AndroidManifest.xml
│   │   ├── build.gradle.kts
│   │   └── proguard-rules.pro
│   ├── build.gradle.kts
│   ├── settings.gradle.kts
│   ├── gradlew  /  gradlew.bat
│   └── gradle/wrapper/gradle-wrapper.properties
│
├── src-rust/                    # Rust core compiled as parsec_core.so (JNI)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs               # JNI entry points (init, ipc, pollEvents, …)
│       ├── ipc.rs               # IPC command dispatcher
│       ├── blocker.rs           # Ad / tracker / popup blocker
│       ├── network.rs           # HTTPS upgrade, HTTP/3 detection
│       ├── profile.rs           # History, bookmarks, sessions, prefs persistence
│       ├── sync.rs              # E2E encrypted cross-device sync
│       ├── extension_store.rs   # Chrome extension registry
│       ├── neutron_android.rs   # GPU compositor (wgpu → Vulkan/GLES on Android)
│       └── sdf_rasteriser.rs    # SDF glyph rasteriser for chrome UI text
│
├── gpu-renderer/                # Standalone GPU text renderer (desktop / shared)
│   ├── Cargo.toml
│   └── src/lib.rs
│
└── browser/                     # Desktop browser (Tauri + React + WebKit patches)
    ├── src-tauri/               # Rust Tauri backend
    │   ├── src/
    │   │   ├── main.rs          # Tauri app entry point
    │   │   ├── tab_manager.rs
    │   │   ├── network.rs
    │   │   ├── blocker.rs
    │   │   ├── sync.rs
    │   │   ├── extension_store.rs
    │   │   ├── extension_runtime.rs
    │   │   ├── neutron.rs       # GPU compositor (desktop)
    │   │   ├── neutron_metal.rs # Metal-specific renderer (macOS)
    │   │   ├── neutron_bridge.rs
    │   │   ├── sdf_rasteriser.rs
    │   │   ├── cdp_devtools.rs  # Chrome DevTools Protocol server
    │   │   ├── request_interceptor.rs
    │   │   ├── background_worker.rs
    │   │   ├── profile.rs
    │   │   └── certs.rs
    │   └── Cargo.toml
    ├── src/
    │   ├── ParsecWeb.tsx        # Main React UI (100k lines, full browser chrome)
    │   └── main.tsx
    ├── gpui/                    # GPU-accelerated UI renderer (TypeScript/Rust)
    ├── extensions/              # Chrome extension compatibility layer
    ├── webkit-patches/          # WebKit patches for parsec-specific features
    └── webkit-build/            # WebKit build scripts
```

## Building

### Android

**Prerequisites:**
- Android Studio Hedgehog or later
- Android NDK 26+
- Rust with `aarch64-linux-android` and `x86_64-linux-android` targets
- [rust-android-gradle plugin](https://github.com/mozilla/rust-android-gradle)

```bash
# Install Rust Android targets
rustup target add aarch64-linux-android x86_64-linux-android

# Build (debug)
cd android
./gradlew assembleDebug

# Build (release)
./gradlew assembleRelease
```

The Gradle plugin compiles `src-rust/` via Cargo and copies the resulting
`libparsec_core.so` into the APK's `jniLibs/` automatically.

### Desktop (macOS)

```bash
cd browser
npm install
npm run tauri build
```

Requires: Rust stable, Node 18+, Xcode (macOS).

## Architecture

```
┌───────────────────────────────────────────────────────────────────┐
│  Android                                                           │
│  ┌────────────────────────────────────────────────────────────┐   │
│  │  Kotlin UI (BrowserActivity)                               │   │
│  │  • One WebView per tab (Android system WebView / Chromium) │   │
│  │  • Native toolbar drawn with Android Views                 │   │
│  │  • Tab strip, URL bar, menu, panels                        │   │
│  └──────────────────────┬─────────────────────────────────────┘   │
│                         │ JNI (parsec_core.so)                     │
│  ┌──────────────────────▼─────────────────────────────────────┐   │
│  │  Rust Core (src-rust/)                                     │   │
│  │  • Ad/tracker blocking (shouldBlockResource → WebViewClient)│   │
│  │  • HTTPS upgrade                                           │   │
│  │  • Profile (history, bookmarks, sessions, prefs)           │   │
│  │  • E2E encrypted sync                                      │   │
│  │  • Extension registry                                      │   │
│  │  • Neutron GPU compositor (wgpu → Vulkan/GLES)             │   │
│  │  • IPC dispatcher                                          │   │
│  └────────────────────────────────────────────────────────────┘   │
└───────────────────────────────────────────────────────────────────┘
```

## Privacy

- All ad/tracker blocking is done locally in the Rust core — no cloud filter
- HTTPS-only mode upgrades HTTP connections before the WebView loads them
- Do-Not-Track header sent on all requests (when enabled)
- Incognito tabs never write to the profile database
- Sync uses XChaCha20-Poly1305 + Argon2id — server sees only ciphertext

## License

MIT — see LICENSE.
