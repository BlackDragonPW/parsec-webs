// Top-level build file
plugins {
    id("com.android.application")        version "8.3.0"  apply false
    id("org.jetbrains.kotlin.android")   version "1.9.22" apply false
    // FIX: bumped from 0.9.4 → 0.9.6 (last stable; 0.9.4 fails to resolve on Gradle 8.x)
    id("org.mozilla.rust-android-gradle.rust-android") version "0.9.6" apply false
}
