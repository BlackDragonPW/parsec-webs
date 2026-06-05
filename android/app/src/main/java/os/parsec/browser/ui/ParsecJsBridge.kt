package os.parsec.browser.ui

import android.webkit.JavascriptInterface
import os.parsec.browser.ParsecCore

/**
 * JS bridge for the new-tab page (parsec://newtab).
 * Lets the in-page search box call back into Kotlin without leaving the WebView sandbox.
 */
class ParsecJsBridge(
    private val activity: BrowserActivity,
    private val tabId: String,
) {
    @JavascriptInterface
    fun search(query: String) {
        activity.runOnUiThread {
            activity.navigateFromBridge(tabId, query.trim())
        }
    }

    @JavascriptInterface
    fun openUrl(url: String) {
        activity.runOnUiThread {
            activity.navigateFromBridge(tabId, url.trim())
        }
    }

    @JavascriptInterface
    fun getPrivacyStats(): String = try {
        val resp = ParsecCore.ipc("""{"id":"ntp","cmd":"GetPrivacyStats","args":{}}""")
        com.google.gson.JsonParser.parseString(resp).asJsonObject
            .getAsJsonObject("data")?.toString() ?: "{}"
    } catch (_: Exception) {
        "{}"
    }
}
