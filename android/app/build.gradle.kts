plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.mozilla.rust-android-gradle.rust-android")
    // FIX: kapt required for Room annotation processing
    id("org.jetbrains.kotlin.kapt")
}

android {
    namespace   = "os.parsec.browser"
    compileSdk  = 34

    defaultConfig {
        applicationId   = "os.parsec.browser"
        minSdk          = 26
        targetSdk       = 34
        versionCode     = 13
        versionName     = "1.3.0"

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"

        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
    }

    buildTypes {
        release {
            isMinifyEnabled   = true
            isShrinkResources = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
            signingConfig = signingConfigs.getByName("debug")
        }
        debug {
            isDebuggable = true
        }
    }

    buildFeatures {
        viewBinding  = true
        buildConfig  = true
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    sourceSets["main"].jniLibs.srcDirs("src/main/jniLibs")
}

// ── Rust/Cargo integration ─────────────────────────────────────────────────────
cargo {
    module        = "../src-rust"
    libname       = "parsec_core"
    targets       = listOf("arm64", "x86_64")
    prebuiltToolchains = true
    // Allow CI to override profile with -Pcargo.profile=debug for faster builds
    profile       = (project.findProperty("cargo.profile") as? String) ?: "release"
}

dependencies {
    // ── AndroidX core ─────────────────────────────────────────────────────
    implementation("androidx.core:core-ktx:1.12.0")
    implementation("androidx.appcompat:appcompat:1.6.1")
    implementation("androidx.activity:activity-ktx:1.8.2")
    implementation("androidx.fragment:fragment-ktx:1.6.2")
    implementation("androidx.lifecycle:lifecycle-viewmodel-ktx:2.7.0")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.7.0")
    implementation("com.google.android.material:material:1.11.0")
    implementation("androidx.constraintlayout:constraintlayout:2.1.4")
    implementation("androidx.recyclerview:recyclerview:1.3.2")
    implementation("androidx.swiperefreshlayout:swiperefreshlayout:1.1.0")
    implementation("androidx.coordinatorlayout:coordinatorlayout:1.2.0")
    implementation("androidx.viewpager2:viewpager2:1.0.0")

    // ── Coroutines ────────────────────────────────────────────────────────
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.7.3")

    // ── JSON ──────────────────────────────────────────────────────────────
    implementation("com.google.code.gson:gson:2.10.1")

    // ── Biometric ─────────────────────────────────────────────────────────
    implementation("androidx.biometric:biometric:1.1.0")

    // ── Security ──────────────────────────────────────────────────────────
    implementation("androidx.security:security-crypto:1.1.0-alpha06")

    // ── OkHttp ────────────────────────────────────────────────────────────
    implementation("com.squareup.okhttp3:okhttp:4.12.0")

    // ── Coil (favicons) ───────────────────────────────────────────────────
    implementation("io.coil-kt:coil:2.5.0")

    // ── Room ─────────────────────────────────────────────────────────────
    // FIX: annotationProcessor → kapt (Java annotation processors don't run
    //      on Kotlin sources; kapt is required for Room to generate the _Impl classes)
    implementation("androidx.room:room-runtime:2.6.1")
    implementation("androidx.room:room-ktx:2.6.1")
    kapt("androidx.room:room-compiler:2.6.1")

    // ── DataStore ─────────────────────────────────────────────────────────
    implementation("androidx.datastore:datastore-preferences:1.0.0")

    // ── Window Manager ────────────────────────────────────────────────────
    implementation("androidx.window:window:1.2.0")

    // ── Testing ───────────────────────────────────────────────────────────
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.5")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.1")
}
