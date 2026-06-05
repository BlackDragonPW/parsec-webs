package os.parsec.browser

import android.net.Uri
import android.util.Log
import android.webkit.WebResourceResponse
import androidx.collection.LruCache
import com.google.gson.JsonObject
import com.google.gson.JsonParser
import java.io.ByteArrayInputStream
import java.net.URLEncoder

/**
 * Lightweight navigation helper — subresource intercept is DISABLED globally
 * because shouldInterceptRequest on WebView causes severe jank on all modern sites.
 */
object ResourceBlocker {

    private const val TAG = "ResourceBlocker"

    data class NavDecision(
        val allow: Boolean,
        val redirectUrl: String? = null,
        val reason: String? = null,
    )

    /** Subresource blocking is off — it breaks video sites and causes universal lag. */
    const val SUBRESOURCE_BLOCKING_ENABLED = false

    @Volatile var blockAds = false
    @Volatile var blockTrackers = false
    @Volatile var blockNsfw = false
    @Volatile var httpsOnly = true

    private var adHosts: HashSet<String> = hashSetOf()
    private var trackerHosts: HashSet<String> = hashSetOf()
    private val hostCache = LruCache<String, Boolean>(512)
    @Volatile private var initialized = false

    private val nsfwKeywords = arrayOf("pornhub", "xvideos", "xnxx", "redtube", "youporn")
    private val minerKeywords = arrayOf("coinhive", "cryptoloot", "coin-hive", "minero.cc", "webmr.ru")

    fun initFromRust() {
        if (initialized) return
        try {
            parseBlockLists(ParsecCore.getBlockLists())
            Log.i(TAG, "Loaded ${adHosts.size} ad + ${trackerHosts.size} tracker hosts")
        } catch (e: UnsatisfiedLinkError) {
            Log.w(TAG, "getBlockLists unavailable — using fallback", e)
            initFallback()
        } catch (e: Exception) {
            Log.w(TAG, "Block list load failed — using fallback", e)
            initFallback()
        }
        loadPrefsSafe()
    }

    private fun parseBlockLists(json: String) {
        val root = JsonParser.parseString(json).asJsonObject
        adHosts = root.getAsJsonArray("ads").mapTo(hashSetOf()) { it.asString }
        trackerHosts = root.getAsJsonArray("trackers").mapTo(hashSetOf()) { it.asString }
        hostCache.evictAll()
        initialized = true
    }

    private fun initFallback() {
        adHosts = hashSetOf("doubleclick.net", "googlesyndication.com")
        trackerHosts = hashSetOf("google-analytics.com")
        hostCache.evictAll()
        initialized = true
    }

    private fun loadPrefsSafe() {
        try {
            val resp = JsonParser.parseString(
                ParsecCore.ipc("""{"id":"0","cmd":"GetPrefs","args":{}}""")
            ).asJsonObject
            if (resp.get("ok")?.asBoolean == true) {
                resp.getAsJsonObject("data")?.let { refreshPrefs(it) }
            }
        } catch (e: Throwable) {
            Log.w(TAG, "Could not load prefs", e)
        }
    }

    fun refreshPrefs(prefsJson: JsonObject) {
        // Subresource prefs kept for settings UI but intercept stays disabled for perf
        prefsJson.get("block_ads")?.asBoolean?.let { blockAds = it }
        prefsJson.get("block_trackers")?.asBoolean?.let { blockTrackers = it }
        prefsJson.get("block_nsfw")?.asBoolean?.let { blockNsfw = it }
        prefsJson.get("https_only")?.asBoolean?.let { httpsOnly = it }
        hostCache.evictAll()
    }

    /** Navigation: HTTPS upgrade only — never block main-frame loads (breaks sites). */
    fun checkNavigation(url: String): NavDecision {
        if (url.startsWith("http://") && httpsOnly) {
            return NavDecision(allow = false, redirectUrl = "https://${url.removePrefix("http://")}")
        }
        return NavDecision(allow = true)
    }

    fun buildSearchUrl(query: String, engine: SearchEngine = SearchEngine.GOOGLE): String {
        val q = URLEncoder.encode(query.trim(), "UTF-8")
        return when (engine) {
            SearchEngine.GOOGLE -> "https://www.google.com/search?q=$q"
            SearchEngine.DUCKDUCKGO -> "https://duckduckgo.com/?q=$q"
            SearchEngine.BING -> "https://www.bing.com/search?q=$q"
        }
    }

    enum class SearchEngine { GOOGLE, DUCKDUCKGO, BING }

    fun blockedResponse(): WebResourceResponse =
        WebResourceResponse("text/plain", "utf-8", ByteArrayInputStream(ByteArray(0)))

    /** Not used while SUBRESOURCE_BLOCKING_ENABLED is false. */
    fun checkSubresource(pageUrl: String?, requestUrl: String): String? {
        if (!SUBRESOURCE_BLOCKING_ENABLED) return null
        if (!initialized) initFromRust()
        if (!blockAds && !blockTrackers && !blockNsfw) return null
        val reqHost = extractHost(requestUrl) ?: return null
        val pageHost = pageUrl?.let { extractHost(it) }
        if (pageHost != null && pageHost == reqHost) return null
        return checkHost(reqHost)
    }

    fun isTrustedHost(@Suppress("UNUSED_PARAMETER") host: String) = true

    private fun checkHost(host: String): String? {
        val h = host.lowercase()
        if (blockAds && isBlockedHost(h, adHosts)) return "ads"
        if (blockTrackers && isBlockedHost(h, trackerHosts)) return "trackers"
        if (blockNsfw && nsfwKeywords.any { h.contains(it) }) return "nsfw"
        if (minerKeywords.any { h.contains(it) }) return "miners"
        return null
    }

    private fun isBlockedHost(host: String, blocked: Set<String>): Boolean {
        if (host in blocked) return true
        var rest = host
        while (true) {
            val dot = rest.indexOf('.')
            if (dot < 0) break
            rest = rest.substring(dot + 1)
            if (rest in blocked) return true
        }
        return false
    }

    private fun extractHost(url: String): String? = try {
        Uri.parse(url).host?.lowercase()
    } catch (_: Exception) {
        null
    }
}
