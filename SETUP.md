# Parsec Browser Android — Build Setup

## Requirements

| Tool | Version | Notes |
|------|---------|-------|
| JDK | 17+ | Temurin/OpenJDK both work |
| Android SDK | API 34 (compile), API 26+ (run) | Via Android Studio or `sdkmanager` |
| Android NDK | r26 (`26.3.11579264`) | Install via SDK Manager |
| Rust | 1.78+ stable | Via [rustup](https://rustup.rs) |
| Cargo Android targets | `aarch64-linux-android`, `x86_64-linux-android` | See below |

---

## One-time local setup

### 1. Add Rust Android targets

```bash
rustup target add aarch64-linux-android x86_64-linux-android
```

### 2. Configure Cargo linkers

Add to `~/.cargo/config.toml` (create it if it doesn't exist).  
Replace `$NDK` with your actual NDK path (e.g. `~/Library/Android/sdk/ndk/26.3.11579264`):

```toml
[target.aarch64-linux-android]
linker = "$NDK/toolchains/llvm/prebuilt/darwin-x86_64/bin/aarch64-linux-android26-clang"
ar     = "$NDK/toolchains/llvm/prebuilt/darwin-x86_64/bin/llvm-ar"

[target.x86_64-linux-android]
linker = "$NDK/toolchains/llvm/prebuilt/darwin-x86_64/bin/x86_64-linux-android26-clang"
ar     = "$NDK/toolchains/llvm/prebuilt/darwin-x86_64/bin/llvm-ar"
```

On Linux replace `darwin-x86_64` with `linux-x86_64`.  
On Windows replace `darwin-x86_64` with `windows-x86_64` and use `.cmd` extensions.

### 3. Configure local SDK path

```bash
cp android/local.properties.example android/local.properties
# Edit android/local.properties with your sdk.dir
```

### 4. Bootstrap gradle-wrapper.jar

This binary file is not stored in git. Generate it once:

```bash
cd android
# Option A — if you have Gradle installed globally:
gradle wrapper --gradle-version 8.6

# Option B — download the official jar directly:
curl -sL "https://services.gradle.org/distributions/gradle-8.6-wrapper.jar" \
     -o gradle/wrapper/gradle-wrapper.jar
# Verify:
echo "4edeb139d6cfa4c0467bab5ae11df1d492e1babf1dfd7ef846511242d8fbd122  gradle/wrapper/gradle-wrapper.jar" | sha256sum -c
```

---

## Build a debug APK

```bash
cd android
chmod +x gradlew
./gradlew assembleDebug
# Output: app/build/outputs/apk/debug/app-debug.apk
```

## Check Rust crate only (fast)

```bash
cd src-rust
cargo check --target aarch64-linux-android
```

## Build Rust library only

```bash
cd src-rust
cargo build --target aarch64-linux-android --release
```

---

## CI (GitHub Actions)

Push to `main`, `master`, or `develop` — the workflow at  
`.github/workflows/build.yml` runs automatically:

1. Bootstraps `gradle-wrapper.jar` from the official Gradle CDN (checksum verified)
2. Installs JDK 17, Android SDK, NDK r26
3. Installs Rust + Android targets
4. Runs `cargo check` on the Rust crate
5. Runs `./gradlew assembleDebug`
6. Uploads `app-debug.apk` as a workflow artifact (retained 30 days)

---

## Release signing (optional)

The current `build.gradle.kts` uses the debug signing key for release builds  
(safe for sideloading, not for Play Store).

To sign for distribution, add a `release` signing config:

```kotlin
// In android/app/build.gradle.kts
android {
    signingConfigs {
        create("release") {
            storeFile = file(System.getenv("KEYSTORE_PATH") ?: "release.jks")
            storePassword = System.getenv("KEYSTORE_PASSWORD")
            keyAlias = System.getenv("KEY_ALIAS")
            keyPassword = System.getenv("KEY_PASSWORD")
        }
    }
    buildTypes {
        release {
            signingConfig = signingConfigs.getByName("release")
        }
    }
}
```

Then set `KEYSTORE_PATH`, `KEYSTORE_PASSWORD`, `KEY_ALIAS`, `KEY_PASSWORD`  
as GitHub Actions secrets and reference them in the workflow env block.
