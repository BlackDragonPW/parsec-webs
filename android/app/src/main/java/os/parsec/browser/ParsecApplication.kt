package os.parsec.browser

import android.app.Application
import android.content.pm.ApplicationInfo
import android.webkit.WebView
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob

class ParsecApplication : Application() {

    val applicationScope = CoroutineScope(SupervisorJob() + Dispatchers.Main)

    override fun onCreate() {
        super.onCreate()

        // Enable remote WebView debugging in debug builds
        if (applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0) {
            WebView.setWebContentsDebuggingEnabled(true)
        }

        // Load Rust JNI library
        System.loadLibrary("parsec_core")

        // Initialise Parsec core with the app's private files directory
        ParsecCore.init(filesDir.absolutePath)
    }
}
