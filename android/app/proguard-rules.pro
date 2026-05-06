# Parsec Browser ProGuard rules

# ── Keep JNI entry point class ─────────────────────────────────────────────────
-keep class os.parsec.browser.ParsecCore { *; }
-keep class os.parsec.browser.ParsecApplication { *; }

# ── Keep all browser UI classes (referenced from XML / bottom sheets) ──────────
-keep class os.parsec.browser.ui.** { *; }
-keep class os.parsec.browser.adapter.** { *; }
-keep class os.parsec.browser.service.** { *; }

# ── Gson: keep all data classes that are serialised/deserialised ───────────────
-keepattributes Signature
-keepattributes *Annotation*
-keep class com.google.gson.** { *; }
-keep class * implements com.google.gson.TypeAdapterFactory
-keep class * implements com.google.gson.JsonSerializer
-keep class * implements com.google.gson.JsonDeserializer
# Keep all fields used by Gson reflection
-keepclassmembers class * {
    @com.google.gson.annotations.SerializedName <fields>;
}

# ── Room: keep generated _Impl classes ────────────────────────────────────────
-keep class * extends androidx.room.RoomDatabase { *; }
-keep @androidx.room.Entity class *
-keep @androidx.room.Dao interface *
-dontwarn androidx.room.**

# ── OkHttp ────────────────────────────────────────────────────────────────────
-dontwarn okhttp3.**
-dontwarn okio.**
-keep class okhttp3.** { *; }
-keep interface okhttp3.** { *; }

# ── Coil ──────────────────────────────────────────────────────────────────────
-dontwarn coil.**

# ── WebView JavaScript interface ──────────────────────────────────────────────
-keepclassmembers class * {
    @android.webkit.JavascriptInterface <methods>;
}

# ── Kotlin coroutines ─────────────────────────────────────────────────────────
-keepclassmembernames class kotlinx.** {
    volatile <fields>;
}
-dontwarn kotlinx.coroutines.**

# ── AndroidX ──────────────────────────────────────────────────────────────────
-dontwarn androidx.**
-keep class androidx.core.app.NotificationCompat$Builder { *; }

# ── General: keep line numbers for crash reports ──────────────────────────────
-keepattributes SourceFile,LineNumberTable
-renamesourcefileattribute SourceFile
