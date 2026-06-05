package os.parsec.browser

import android.app.Application
import android.content.pm.ApplicationInfo
import android.util.Log
import android.webkit.WebView
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob

class ParsecApplication : Application() {

    val applicationScope = CoroutineScope(SupervisorJob() + Dispatchers.Main)

    override fun onCreate() {
        super.onCreate()

        if (applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0) {
            WebView.setWebContentsDebuggingEnabled(true)
        }

        try {
            System.loadLibrary("parsec_core")
            ParsecCore.init(filesDir.absolutePath)
            ResourceBlocker.initFromRust()
        } catch (e: UnsatisfiedLinkError) {
            Log.e(TAG, "Native library failed to load", e)
            ResourceBlocker.initFromRust() // still sets up Kotlin fallback lists
        } catch (e: Exception) {
            Log.e(TAG, "Parsec core init failed", e)
            ResourceBlocker.initFromRust()
        }
    }

    companion object {
        private const val TAG = "ParsecApplication"
    }
}
