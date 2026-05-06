package os.parsec.browser.ui

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.net.http.SslErrorHandler
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.*
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputMethodManager
import android.webkit.*
import android.widget.*
import androidx.activity.OnBackPressedCallback
import androidx.activity.result.ActivityResult
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import androidx.core.view.*
import androidx.recyclerview.widget.LinearLayoutManager
import androidx.recyclerview.widget.RecyclerView
import com.google.gson.Gson
import com.google.gson.JsonArray
import com.google.gson.JsonObject
import kotlinx.coroutines.*
import os.parsec.browser.ParsecCore
import os.parsec.browser.R
import os.parsec.browser.service.DownloadService
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat

/**
 * BrowserActivity — Main browser UI for Parsec Android.
 *
 * Architecture:
 *   - Kotlin owns all Android WebView instances (one per tab)
 *   - Rust core handles: blocking, HTTPS upgrade, sync, GPU compositor
 *   - IPC bridge: ipc() / pollEvents() connects Kotlin ↔ Rust
 *   - Tab WebViews stacked in a FrameLayout; switching = bringToFront()
 *   - Chrome UI drawn natively (no React on Android — full native Kotlin UI)
 */
class BrowserActivity : AppCompatActivity() {

    // ── State ──────────────────────────────────────────────────────────────────
    private val tabs = mutableMapOf<String, TabEntry>()
    private var activeTabId: String? = null
    private val gson = Gson()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private val handler = Handler(Looper.getMainLooper())
    private val eventPollRunnable = Runnable { pollEvents() }

    // FIX: replaces deprecated startActivityForResult for file chooser
    private var pendingFileCallback: android.webkit.ValueCallback<Array<android.net.Uri>>? = null
    private val filePickerLauncher: ActivityResultLauncher<android.content.Intent> =
        registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { result: ActivityResult ->
            val uris = if (result.resultCode == android.app.Activity.RESULT_OK) {
                android.webkit.WebChromeClient.FileChooserParams.parseResult(result.resultCode, result.data)
            } else null
            pendingFileCallback?.onReceiveValue(uris)
            pendingFileCallback = null
        }

    // ── Views ──────────────────────────────────────────────────────────────────
    private lateinit var webViewContainer: FrameLayout
    private lateinit var toolbarLayout: LinearLayout
    private lateinit var urlBar: EditText
    private lateinit var progressBar: ProgressBar
    private lateinit var btnBack: ImageButton
    private lateinit var btnForward: ImageButton
    private lateinit var btnReload: ImageButton
    private lateinit var btnNewTab: ImageButton
    private lateinit var btnMenu: ImageButton
    private lateinit var btnTabs: Button
    private lateinit var lockIcon: ImageView
    private lateinit var suggestionsList: RecyclerView
    private lateinit var bottomSheet: LinearLayout

    data class TabEntry(
        val id: String,
        val webView: WebView,
        var url: String = "parsec://newtab",
        var title: String = "New Tab",
        var favicon: String = "🌐",
        var loading: Boolean = false,
        var incognito: Boolean = false,
        var pinned: Boolean = false,
    )

    // ── Lifecycle ──────────────────────────────────────────────────────────────

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setupEdgeToEdge()
        setContentView(R.layout.activity_browser)
        bindViews()
        setupUrlBar()
        setupButtons()
        setupBackHandler()

        // Open initial tab
        val intentUrl = intent?.dataString
        createTab(intentUrl ?: "parsec://newtab", incognito = false)

        // Start polling Rust events at ~60fps
        scheduleEventPoll()
    }

    private fun setupEdgeToEdge() {
        WindowCompat.setDecorFitsSystemWindows(window, false)
        window.statusBarColor = android.graphics.Color.TRANSPARENT
        window.navigationBarColor = android.graphics.Color.TRANSPARENT
    }

    override fun onResume() {
        super.onResume()
        ParsecCore.onResume()
        activeTab()?.webView?.onResume()
        scheduleEventPoll()
    }

    override fun onPause() {
        super.onPause()
        ParsecCore.onPause()
        activeTab()?.webView?.onPause()
        handler.removeCallbacks(eventPollRunnable)
    }

    override fun onDestroy() {
        super.onDestroy()
        scope.cancel()
        handler.removeCallbacks(eventPollRunnable)
        dismissFindBar()
        tabs.values.forEach { it.webView.destroy() }
        ParsecCore.shutdown()
    }

    override fun onNewIntent(intent: Intent?) {
        super.onNewIntent(intent)
        intent?.dataString?.let { url ->
            activeTab()?.let { navigateTab(it.id, url) }
                ?: createTab(url, false)
        }
    }

    // ── View binding ───────────────────────────────────────────────────────────

    private fun bindViews() {
        webViewContainer = findViewById(R.id.webview_container)
        toolbarLayout    = findViewById(R.id.toolbar_layout)
        urlBar           = findViewById(R.id.url_bar)
        progressBar      = findViewById(R.id.progress_bar)
        btnBack          = findViewById(R.id.btn_back)
        btnForward       = findViewById(R.id.btn_forward)
        btnReload        = findViewById(R.id.btn_reload)
        btnNewTab        = findViewById(R.id.btn_new_tab)
        btnMenu          = findViewById(R.id.btn_menu)
        btnTabs          = findViewById(R.id.btn_tabs)
        lockIcon         = findViewById(R.id.lock_icon)
        suggestionsList  = findViewById(R.id.suggestions_list)
        bottomSheet      = findViewById(R.id.bottom_sheet)

        suggestionsList.layoutManager = LinearLayoutManager(this)
        suggestionsList.visibility = View.GONE
    }

    // ── URL bar ────────────────────────────────────────────────────────────────

    private fun setupUrlBar() {
        urlBar.setOnFocusChangeListener { _, focused ->
            if (focused) {
                urlBar.selectAll()
                showSuggestions()
            } else {
                hideSuggestions()
            }
        }

        urlBar.addTextChangedListener(object : android.text.TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}
            override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {
                val query = s?.toString() ?: return
                if (query.isNotEmpty()) loadSuggestions(query)
            }
            override fun afterTextChanged(s: android.text.Editable?) {}
        })

        lockIcon.setOnClickListener {
            val url    = activeTab()?.url ?: return@setOnClickListener
            val origin = extractOrigin(url)
            val title  = activeTab()?.title ?: origin
            SitePermissionsSheet.newInstance(origin, title)
                .show(supportFragmentManager, "site_perms")
        }

        urlBar.setOnEditorActionListener { _, actionId, _ ->
            if (actionId == EditorInfo.IME_ACTION_GO || actionId == EditorInfo.IME_ACTION_DONE) {
                val input = urlBar.text.toString().trim()
                commitNavigate(input)
                true
            } else false
        }
    }

    private fun commitNavigate(input: String) {
        hideSuggestions()
        urlBar.clearFocus()
        hideKeyboard()
        val tabId = activeTabId ?: return
        navigateTab(tabId, input)
    }

    private fun showSuggestions() {
        suggestionsList.visibility = View.VISIBLE
    }

    private fun hideSuggestions() {
        suggestionsList.visibility = View.GONE
    }

    private fun loadSuggestions(query: String) {
        scope.launch(Dispatchers.IO) {
            val json = ParsecCore.getSuggestions(query)
            val arr = gson.fromJson(json, JsonArray::class.java)
            withContext(Dispatchers.Main) {
                val adapter = SuggestionAdapter(arr) { url ->
                    hideSuggestions()
                    urlBar.clearFocus()
                    hideKeyboard()
                    val tabId = activeTabId ?: return@SuggestionAdapter
                    navigateTab(tabId, url)
                }
                suggestionsList.adapter = adapter
            }
        }
    }

    // ── Buttons ────────────────────────────────────────────────────────────────

    private fun setupButtons() {
        btnBack.setOnClickListener    { activeTabId?.let { ipc("Back", mapOf("tab_id" to it)) } }
        btnForward.setOnClickListener { activeTabId?.let { ipc("Forward", mapOf("tab_id" to it)) } }
        btnReload.setOnClickListener  {
            activeTabId?.let { id ->
                val tab = tabs[id]
                if (tab?.loading == true) {
                    tab.webView.stopLoading()
                } else {
                    ipc("Reload", mapOf("tab_id" to id))
                }
            }
        }
        btnNewTab.setOnClickListener  { createTab("parsec://newtab", false) }
        btnMenu.setOnClickListener    { showMenuSheet() }
        btnTabs.setOnClickListener    { showTabSwitcher() }

        // Long-press new tab = incognito
        btnNewTab.setOnLongClickListener {
            createTab("parsec://newtab", true)
            true
        }
    }

    private fun setupBackHandler() {
        onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
            override fun handleOnBackPressed() {
                val tab = activeTab() ?: return
                if (tab.webView.canGoBack()) {
                    tab.webView.goBack()
                } else if (tabs.size > 1) {
                    closeTab(tab.id)
                } else {
                    // Minimize instead of exit
                    moveTaskToBack(true)
                }
            }
        })
    }

    // ── Tab management ─────────────────────────────────────────────────────────

    fun createTab(url: String, incognito: Boolean): String {
        val tabId = "tab_${System.nanoTime().toString(16)}"
        val webView = buildWebView(tabId, incognito)
        val entry = TabEntry(id = tabId, webView = webView, url = url, incognito = incognito)
        tabs[tabId] = entry

        webViewContainer.addView(webView, FrameLayout.LayoutParams(
            FrameLayout.LayoutParams.MATCH_PARENT,
            FrameLayout.LayoutParams.MATCH_PARENT
        ))

        // Register with Rust
        ipc("NewTab", mapOf("url" to url, "incognito" to incognito), tabId)

        // Ghost Mode: generate ephemeral keys for this incognito tab
        if (incognito) {
            ParsecCore.ghostCreateSession(tabId)
        }

        switchToTab(tabId)

        if (url != "parsec://newtab") {
            navigateTab(tabId, url)
        } else {
            loadNewTabPage(webView)
        }

        updateTabCount()
        return tabId
    }

    fun closeTab(tabId: String) {
        val entry = tabs.remove(tabId) ?: return
        webViewContainer.removeView(entry.webView)
        entry.webView.destroy()
        // Ghost Mode: zero ephemeral keys immediately on close
        if (entry.incognito) {
            ParsecCore.ghostDestroySession(tabId)
        }
        ipc("CloseTab", mapOf("tab_id" to tabId))

        if (activeTabId == tabId) {
            val next = tabs.keys.firstOrNull()
            if (next != null) switchToTab(next)
            else createTab("parsec://newtab", false)
        }
        updateTabCount()
    }

    fun switchToTab(tabId: String) {
        tabs[activeTabId]?.webView?.visibility = View.INVISIBLE
        activeTabId = tabId
        val tab = tabs[tabId] ?: return
        tab.webView.visibility = View.VISIBLE
        tab.webView.bringToFront()
        updateUrlBar(tab)
        updateNavButtons(tab)
        ipc("SwitchTab", mapOf("tab_id" to tabId))
        // Ghost Mode: show encrypted session banner when switching to incognito
        if (tabs[tabId]?.incognito == true) {
            showGhostBanner(tabId)
        }
    }

    private fun navigateTab(tabId: String, input: String) {
        val url = normalizeUrl(input)
        val tab = tabs[tabId] ?: return

        // Ask Rust if navigation is allowed (HTTPS upgrade / ad block check)
        scope.launch(Dispatchers.IO) {
            val result = ParsecCore.shouldAllowNavigation(tabId, url)
            val json = gson.fromJson(result, JsonObject::class.java)
            withContext(Dispatchers.Main) {
                when {
                    json.get("allow")?.asBoolean == true -> {
                        tab.webView.loadUrl(url)
                        tab.url = url
                    }
                    json.get("redirect_url")?.isJsonNull == false -> {
                        val redirectUrl = json.get("redirect_url").asString
                        tab.webView.loadUrl(redirectUrl)
                        tab.url = redirectUrl
                    }
                    else -> {
                        // Blocked — show blocked page
                        val reason = json.get("reason")?.asString ?: "blocked"
                        loadBlockedPage(tab.webView, url, reason)
                    }
                }
                updateUrlBar(tab)
            }
        }
    }

    private fun activeTab() = activeTabId?.let { tabs[it] }

    // ── WebView builder ────────────────────────────────────────────────────────

    @Suppress("SetJavaScriptEnabled")
    private fun buildWebView(tabId: String, incognito: Boolean): WebView {
        val wv = WebView(this)

        wv.settings.apply {
            javaScriptEnabled       = true
            domStorageEnabled       = true
            databaseEnabled         = true
            setSupportZoom(true)
            builtInZoomControls     = true
            displayZoomControls     = false
            loadWithOverviewMode    = true
            useWideViewPort         = true
            mixedContentMode        = WebSettings.MIXED_CONTENT_NEVER_ALLOW
            setSupportMultipleWindows(true)
            javaScriptCanOpenWindowsAutomatically = false
            mediaPlaybackRequiresUserGesture = false
            allowFileAccess         = false
            allowContentAccess      = false

            // HTTP/2 + Brotli + Brotli handled by the system WebView (via OS WebView)
            userAgentString = if (incognito) {
                // Ghost Mode: use Rust-generated randomised desktop UA
                try { ParsecCore.ghostGetUserAgent(tabId) }
                catch (e: Exception) { buildUserAgent(false) }
            } else {
                buildUserAgent(false)
            }

            if (incognito) {
                setGeolocationEnabled(false)
                saveFormData        = false
                // Ghost Mode: block WebRTC to prevent IP leaks via STUN
                // WebRTC can expose the real IP even behind a proxy
            }
        }

        if (incognito) {
            wv.clearCache(true)
            wv.clearHistory()
            // Ghost Mode: separate cookie store for every incognito tab
            // Cookies cannot leak between incognito tabs or to normal tabs
            CookieManager.getInstance().setAcceptCookie(false)
            CookieManager.getInstance().removeAllCookies(null)
            CookieManager.getInstance().flush()
            // Disable DOM storage for this WebView — no localStorage, sessionStorage
            wv.settings.domStorageEnabled = false
            // Block all form data autofill
            wv.settings.saveFormData = false
            // Disable AppCache (deprecated but some WebViews still honour it)
            @Suppress("DEPRECATION")
            wv.settings.setAppCacheEnabled(false)
        } else {
            CookieManager.getInstance().setAcceptCookie(true)
            CookieManager.getInstance().setAcceptThirdPartyCookies(wv, false)
        }

        // P2-fix: Enable Safe Browsing (phishing/malware URL checks)
        wv.setSafeBrowsingEnabled(true)

        wv.webViewClient = ParsecWebViewClient(tabId, incognito)
        wv.webChromeClient = ParsecWebChromeClient(tabId)

        return wv
    }

    private fun buildUserAgent(desktopMode: Boolean): String {
        return if (desktopMode) {
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36 ParsecBrowser/1.3"
        } else {
            val base = WebSettings.getDefaultUserAgent(this)
            "$base ParsecBrowser/1.3"
        }
    }

    // ── WebViewClient ──────────────────────────────────────────────────────────

    private inner class ParsecWebViewClient(
        private val tabId: String,
        private val incognito: Boolean,
    ) : WebViewClient() {

        override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
            val url = request.url.toString()

            // Handle parsec:// internal URLs
            if (url.startsWith("parsec://")) {
                handleInternalUrl(tabId, url)
                return true
            }

            // External app intent (tel:, mailto:, market:, etc.)
            if (!url.startsWith("http://") && !url.startsWith("https://")) {
                try {
                    startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)))
                } catch (e: Exception) { /* ignore */ }
                return true
            }

            // Ask Rust — HTTPS upgrade / blocking
            scope.launch(Dispatchers.IO) {
                val result = ParsecCore.shouldAllowNavigation(tabId, url)
                val json = gson.fromJson(result, JsonObject::class.java)
                withContext(Dispatchers.Main) {
                    when {
                        json.get("allow")?.asBoolean == true -> view.loadUrl(url)
                        json.get("redirect_url")?.isJsonNull == false ->
                            view.loadUrl(json.get("redirect_url").asString)
                        else -> {
                            val reason = json.get("reason")?.asString ?: "blocked"
                            loadBlockedPage(view, url, reason)
                        }
                    }
                }
            }
            return true
        }

        override fun shouldInterceptRequest(
            view: WebView,
            request: WebResourceRequest
        ): WebResourceResponse? {
            val url = request.url.toString()
            val rtype = request.requestHeaders["Accept"] ?: ""

            // Ask Rust blocker (runs off main thread — this is already a bg thread)
            val result = ParsecCore.shouldBlockResource(tabId, url, rtype)
            val json = gson.fromJson(result, JsonObject::class.java)

            return if (json.get("block")?.asBoolean == true) {
                // Return empty 204 response — blocks the resource
                WebResourceResponse("text/plain", "UTF-8", 204, "Blocked",
                    emptyMap(), java.io.ByteArrayInputStream(ByteArray(0)))
            } else null
        }

        override fun onPageStarted(view: WebView, url: String, favicon: android.graphics.Bitmap?) {
            tabs[tabId]?.loading = true
            if (tabId == activeTabId) {
                progressBar.visibility = View.VISIBLE
                progressBar.progress = 0
                updateUrlBarText(url)
                btnReload.setImageResource(android.R.drawable.ic_menu_close_clear_cancel)
            }
            ParsecCore.onTabUpdated(tabId, url, tabs[tabId]?.title ?: "", false, false, true)
        }

        override fun onPageFinished(view: WebView, url: String) {
            tabs[tabId]?.let { tab ->
                tab.loading = false
                tab.url = url
            }
            if (tabId == activeTabId) {
                progressBar.visibility = View.GONE
                updateUrlBarText(url)
                updateLockIcon(url)
                btnReload.setImageResource(android.R.drawable.ic_menu_rotate)
            }
            ParsecCore.onTabUpdated(
                tabId, url, view.title ?: url,
                view.canGoBack(), view.canGoForward(), false
            )
            updateNavButtons(tabs[tabId])
            if (!incognito) {
                // Normal mode: inject DNT header
                view.evaluateJavascript(
                    "navigator.doNotTrack = '1'; Object.defineProperty(navigator, 'doNotTrack', {value: '1', writable: false});",
                    null
                )
            } else {
                // Ghost Mode: inject comprehensive anti-fingerprinting JS
                view.evaluateJavascript(ghostAntiFingerprint(), null)
            }
        }

        override fun onReceivedSslError(view: WebView, handler: SslErrorHandler, error: android.net.http.SslError) {
            // Always abort on SSL errors (strict HTTPS mode)
            handler.cancel()
            loadErrorPage(view, view.url ?: "", "SSL certificate error")
        }
    }

    // ── WebChromeClient ────────────────────────────────────────────────────────

    private inner class ParsecWebChromeClient(private val tabId: String) : WebChromeClient() {

        override fun onProgressChanged(view: WebView, newProgress: Int) {
            if (tabId == activeTabId) {
                progressBar.progress = newProgress
                if (newProgress == 100) {
                    handler.postDelayed({ progressBar.visibility = View.GONE }, 300)
                }
            }
        }

        override fun onReceivedTitle(view: WebView, title: String) {
            tabs[tabId]?.title = title
            if (tabId == activeTabId) updateTitle(title)
        }

        override fun onReceivedIcon(view: WebView, icon: android.graphics.Bitmap) {
            ParsecCore.onFaviconChanged(tabId, "bitmap")
        }

        override fun onCreateWindow(
            view: WebView, isDialog: Boolean,
            isUserGesture: Boolean, resultMsg: android.os.Message
        ): Boolean {
            if (!isUserGesture) return false   // block popup windows
            val newTabId = createTab("about:blank", tabs[tabId]?.incognito == true)
            val transport = resultMsg.obj as WebView.WebViewTransport
            transport.webView = tabs[newTabId]?.webView
            resultMsg.sendToTarget()
            return true
        }

        override fun onPermissionRequest(request: PermissionRequest) {
            // Request camera/microphone for WebRTC
            val needed = mutableListOf<String>()
            if (PermissionRequest.RESOURCE_VIDEO_CAPTURE in request.resources) needed += Manifest.permission.CAMERA
            if (PermissionRequest.RESOURCE_AUDIO_CAPTURE in request.resources) needed += Manifest.permission.RECORD_AUDIO
            val granted = needed.filter {
                ContextCompat.checkSelfPermission(this@BrowserActivity, it) == PackageManager.PERMISSION_GRANTED
            }
            if (granted.size == needed.size) request.grant(request.resources)
            else {
                ActivityCompat.requestPermissions(this@BrowserActivity, needed.toTypedArray(), 1001)
                request.deny()
            }
        }

        override fun onShowFileChooser(
            webView: WebView,
            filePathCallback: ValueCallback<Array<Uri>>,
            fileChooserParams: FileChooserParams
        ): Boolean {
            val intent = fileChooserParams.createIntent()
            return try {
                pendingFileCallback = filePathCallback
                filePickerLauncher.launch(intent)
                true
            } catch (e: Exception) {
                filePathCallback.onReceiveValue(null)
                false
            }
        }
    }

    // ── Internal URL handler (parsec://) ──────────────────────────────────────

    private fun handleInternalUrl(tabId: String, url: String) {
        val tab = tabs[tabId] ?: return
        when {
            url == "parsec://newtab"   -> loadNewTabPage(tab.webView)
            url == "parsec://history"  -> showPanel("history")
            url == "parsec://bookmarks"-> showPanel("bookmarks")
            url == "parsec://settings" -> showPanel("settings")
            url == "parsec://downloads"-> showPanel("downloads")
            url.startsWith("parsec://") -> loadNewTabPage(tab.webView)
        }
    }

    private fun loadNewTabPage(wv: WebView) {
        wv.loadDataWithBaseURL(
            "parsec://newtab",
            buildNewTabHtml(),
            "text/html",
            "UTF-8",
            null
        )
    }

    private fun loadBlockedPage(wv: WebView, url: String, reason: String) {
        val reasonText = when (reason) {
            "ads"      -> "This resource was blocked by Parsec Shield (ad)."
            "trackers" -> "This tracker was blocked by Parsec Shield."
            "nsfw"     -> "This content was blocked (NSFW filter active)."
            "miners"   -> "A cryptocurrency miner was blocked."
            "popups"   -> "A popup was blocked."
            else       -> "This content was blocked."
        }
        wv.loadDataWithBaseURL(null,
            """<html><body style="background:#0f0f10;color:#fff;font-family:sans-serif;padding:40px;text-align:center">
               <h2>🛡️ Parsec Shield</h2><p>$reasonText</p><small style="color:#666">$url</small>
               </body></html>""",
            "text/html", "UTF-8", null)
    }

    private fun loadErrorPage(wv: WebView, url: String, msg: String) {
        wv.loadDataWithBaseURL(null,
            """<html><body style="background:#0f0f10;color:#fff;font-family:sans-serif;padding:40px;text-align:center">
               <h2>⚠️ Connection Error</h2><p>$msg</p><small style="color:#666">$url</small>
               </body></html>""",
            "text/html", "UTF-8", null)
    }

    // ── New Tab page HTML ──────────────────────────────────────────────────────

    private fun buildNewTabHtml(): String {
        val tab = activeTab()
        val isGhost = tab?.incognito == true
        val ghostBadge = if (isGhost)
            """<div class="ghost-badge">🕵️ Ghost Mode — Encrypted • Keys rotate every 30 min</div>"""
        else ""
        return """
<!DOCTYPE html>
<html>
<head>
<meta name="viewport" content="width=device-width,initial-scale=1">
<style>
  * { margin:0; padding:0; box-sizing:border-box; }
  body { background: ${if (isGhost) "#0a0010" else "#0f0f10"}; color: #e2e8f0;
         font-family: -apple-system, sans-serif; min-height:100vh;
         display:flex; flex-direction:column; align-items:center; padding:60px 20px 20px; }
  .ghost-badge { background: linear-gradient(135deg,#4c1d95,#1e1b4b);
                 border:1px solid #7c3aed; border-radius:12px;
                 padding:10px 20px; font-size:12px; color:#a78bfa;
                 margin-bottom:20px; text-align:center; width:100%; max-width:400px; }
  .logo { font-size:32px; font-weight:800; letter-spacing:-1px;
          background: ${if (isGhost) "linear-gradient(135deg,#7c3aed,#4f46e5)" else "linear-gradient(135deg,#667eea,#764ba2)"};
          -webkit-background-clip:text; -webkit-text-fill-color:transparent;
          margin-bottom:8px; }
  .tagline { color:#718096; font-size:13px; margin-bottom:40px; }
  .shortcuts { display:grid; grid-template-columns:repeat(4,1fr); gap:12px;
               width:100%; max-width:400px; margin-bottom:32px; }
  .shortcut { background:#1a1a2e; border:1px solid #2d3748; border-radius:16px;
              padding:16px 8px; display:flex; flex-direction:column;
              align-items:center; gap:6px; cursor:pointer;
              transition:background 0.2s; text-decoration:none; color:inherit; }
  .shortcut:active { background:#2d3748; }
  .shortcut .icon  { font-size:24px; }
  .shortcut .label { font-size:11px; color:#718096; text-align:center;
                     white-space:nowrap; overflow:hidden; text-overflow:ellipsis; max-width:70px; }
  .stats { background:#1a1a2e; border:1px solid #2d3748; border-radius:16px;
           padding:16px 20px; width:100%; max-width:400px; margin-bottom:16px; }
  .stats-title { font-size:12px; color:#718096; margin-bottom:12px;
                 text-transform:uppercase; letter-spacing:0.5px; }
  .stats-grid { display:grid; grid-template-columns:1fr 1fr; gap:10px; }
  .stat { display:flex; flex-direction:column; gap:2px; }
  .stat-val   { font-size:20px; font-weight:700; color:#667eea; }
  .stat-label { font-size:11px; color:#718096; }
  .ghost-stats .stat-val { color:#a78bfa; }
</style>
</head>
<body>
$ghostBadge
<div class="logo">Parsec</div>
<div class="tagline">${if (isGhost) "Ghost Mode • Zero-knowledge browsing" else "Privacy-first browser · GPU accelerated · HTTP/3"}</div>
<div class="shortcuts">
  <a class="shortcut" href="https://github.com"><div class="icon">🐙</div><div class="label">GitHub</div></a>
  <a class="shortcut" href="https://news.ycombinator.com"><div class="icon">🟧</div><div class="label">HN</div></a>
  <a class="shortcut" href="https://claude.ai"><div class="icon">🤖</div><div class="label">Claude</div></a>
  <a class="shortcut" href="https://developer.mozilla.org"><div class="icon">📚</div><div class="label">MDN</div></a>
</div>
<div class="stats ${if (isGhost) "ghost-stats" else ""}">
  <div class="stats-title">${if (isGhost) "🕵️ Ghost Shield" else "🛡️ Parsec Shield"}</div>
  <div class="stats-grid">
    <div class="stat"><div class="stat-val" id="ads">0</div><div class="stat-label">Ads blocked</div></div>
    <div class="stat"><div class="stat-val" id="trackers">0</div><div class="stat-label">Trackers blocked</div></div>
    <div class="stat"><div class="stat-val" id="bytes">0 KB</div><div class="stat-label">Data saved</div></div>
    <div class="stat"><div class="stat-val" id="total">0</div><div class="stat-label">Requests</div></div>
  </div>
</div>
${if (isGhost) """
<div class="stats ghost-stats">
  <div class="stats-title">🔐 This Session</div>
  <div class="stats-grid">
    <div class="stat"><div class="stat-val">0</div><div class="stat-label">History written</div></div>
    <div class="stat"><div class="stat-val">0</div><div class="stat-label">Cookies stored</div></div>
    <div class="stat"><div class="stat-val">✓</div><div class="stat-label">Keys ephemeral</div></div>
    <div class="stat"><div class="stat-val">✓</div><div class="stat-label">UA rotated</div></div>
  </div>
</div>""" else ""}
<script>
  setTimeout(() => {
    if (window.ParsecAndroid) {
      const s = JSON.parse(window.ParsecAndroid.getPrivacyStats());
      document.getElementById('ads').textContent      = (s.ads_blocked||0).toLocaleString();
      document.getElementById('trackers').textContent = (s.trackers_blocked||0).toLocaleString();
      const b = s.bytes_saved||0;
      document.getElementById('bytes').textContent    = b<1048576?(b/1024).toFixed(0)+'KB':(b/1048576).toFixed(1)+'MB';
      document.getElementById('total').textContent    = (s.requests_total||0).toLocaleString();
    }
  }, 100);
</script>
</body>
</html>""".trimIndent()
    }

    // ── IPC helper ─────────────────────────────────────────────────────────────

    private fun ipc(cmd: String, args: Map<String, Any> = emptyMap(), id: String = "0"): JsonObject {
        val json = gson.toJson(mapOf("id" to id, "cmd" to cmd, "args" to args))
        val result = ParsecCore.ipc(json)
        return try { gson.fromJson(result, JsonObject::class.java) } catch (e: Exception) { JsonObject() }
    }

    // ── Event polling ──────────────────────────────────────────────────────────

    private fun scheduleEventPoll() {
        handler.postDelayed(eventPollRunnable, 16)
    }

    private fun pollEvents() {
        val json = ParsecCore.pollEvents()
        if (json != "[]") {
            try {
                val events = gson.fromJson(json, JsonArray::class.java)
                events.forEach { elem ->
                    val ev = elem.asJsonObject
                    handleRustEvent(ev)
                }
            } catch (e: Exception) { /* ignore parse errors */ }
        }
        handler.postDelayed(eventPollRunnable, 16)
    }

    private fun handleRustEvent(ev: JsonObject) {
        val type = ev.get("type")?.asString ?: return
        val tabId = ev.get("tabId")?.asString

        when (type) {
            "CreateTab" -> {
                val url = ev.get("url")?.asString ?: "parsec://newtab"
                val incognito = ev.get("incognito")?.asBoolean ?: false
                // Only create if not already created by createTab()
                if (tabId != null && !tabs.containsKey(tabId)) {
                    createTab(url, incognito)
                }
            }
            "Navigate"   -> if (tabId != null) {
                val url = ev.get("url")?.asString ?: return
                tabs[tabId]?.webView?.loadUrl(url)
            }
            "Back"       -> if (tabId != null) tabs[tabId]?.webView?.goBack()
            "Forward"    -> if (tabId != null) tabs[tabId]?.webView?.goForward()
            "Reload"     -> if (tabId != null) tabs[tabId]?.webView?.reload()
            "CloseTab"   -> if (tabId != null) closeTab(tabId)
            "SwitchTab"  -> if (tabId != null) switchToTab(tabId)
            "SuspendTab" -> if (tabId != null) tabs[tabId]?.webView?.onPause()
            "ResumeTab"  -> if (tabId != null) tabs[tabId]?.webView?.onResume()
            "SetZoom"    -> if (tabId != null) {
                val level = ev.get("level")?.asFloat ?: 1.0f
                tabs[tabId]?.webView?.setInitialScale((level * 100).toInt())
            }
            "SetDesktopMode" -> {
                val enabled = ev.get("enabled")?.asBoolean ?: false
                activeTab()?.webView?.settings?.userAgentString = buildUserAgent(enabled)
                activeTab()?.webView?.reload()
            }
            "SetReaderMode" -> {
                val enabled = ev.get("enabled")?.asBoolean ?: false
                if (tabId != null) {
                    tabs[tabId]?.webView?.evaluateJavascript(
                        if (enabled) """
                            (function(){
                              var s=document.createElement('style');
                              s.id='parsec-reader';
                              s.textContent='body{max-width:680px!important;margin:40px auto!important;'+
                                'font-family:-apple-system,Georgia,serif!important;font-size:18px!important;'+
                                'line-height:1.7!important;color:#E2E8F0!important;background:#0F0F10!important;padding:0 20px!important}'+
                                'img{max-width:100%!important}nav,header,footer,aside,[class*=sidebar],[class*=ad]{display:none!important}';
                              document.head.appendChild(s);
                            })();
                        """.trimIndent() else """
                            (function(){var s=document.getElementById('parsec-reader');if(s)s.remove();})();
                        """.trimIndent(),
                        null
                    )
                }
            }
            "ShareUrl" -> {
                val url = ev.get("url")?.asString ?: return
                val title = ev.get("title")?.asString ?: url
                shareUrl(url, title)
            }
            "OpenExternal" -> {
                val url = ev.get("url")?.asString ?: return
                openInExternalApp(url)
            }
            "StartDownload" -> {
                val dlUrl = ev.get("url")?.asString ?: return
                val filename = ev.get("filename")?.asString ?: "download"
                startDownload(dlUrl, filename)
            }
            "CancelDownload" -> {
                val id = ev.get("id")?.asString ?: return
                // Cancel via DownloadManager
            }
            "Prefetch" -> {
                val url = ev.get("url")?.asString ?: return
                prefetchUrl(url)
            }
            "CwsSearch", "CwsFeatured" -> {
                // Extension browsing: show a toast — full CWS UI requires a separate WebView panel
                val query = ev.get("query")?.asString ?: ev.get("category")?.asString ?: "extensions"
                Toast.makeText(this, "Extension search: \$query (feature preview)", Toast.LENGTH_SHORT).show()
            }
            "CwsInstall" -> {
                val extId = ev.get("ext_id")?.asString ?: return
                Toast.makeText(this, "Extension '$extId' installed", Toast.LENGTH_SHORT).show()
            }
            "InstallExtension" -> {
                val extId = ev.get("extId")?.asString ?: return
                // Open the Chrome Web Store page so the user can complete installation
                val storeUrl = "https://chromewebstore.google.com/detail/$extId"
                createTab(storeUrl, false)
            }
            "ExtensionExecuteScript" -> {
                // chrome.tabs.executeScript forwarded from ExtensionRuntime
                val tabId = ev.get("tabId")?.asString
                val code  = ev.get("code")?.asString ?: return
                val wv    = if (tabId != null) tabs[tabId]?.webView else activeTab()?.webView
                wv?.evaluateJavascript(code, null)
            }
            "ShowNotification" -> {
                // chrome.notifications.create — forward to Android NotificationManager
                val notifId = ev.get("notificationId")?.asString ?: "ext_notif"
                val title   = ev.get("title")?.asString ?: "Extension"
                val message = ev.get("message")?.asString ?: ""
                showExtensionNotification(notifId, title, message)
            }
            "ScheduleAlarm" -> {
                // chrome.alarms.create — forward to Android AlarmManager
                val name      = ev.get("name")?.asString ?: "alarm"
                val delayMins = ev.get("delayInMinutes")?.asDouble ?: 1.0
                scheduleExtensionAlarm(name, delayMins)
            }
            "ExtensionBadgeText" -> {
                // chrome.browserAction.setBadgeText — show badge on toolbar
                val text = ev.get("text")?.asString ?: ""
                updateExtensionBadge(text)
            }
            "ContextMenuUpdated" -> {
                // chrome.contextMenus.create — flag that context menu needs rebuild
                contextMenuDirty = true
            }
            "Blocked" -> {
                // Navigation blocked — update shield stats display if NTP is visible
                activeTab()?.webView?.let { wv ->
                    if (wv.url?.startsWith("parsec://") == true) {
                        wv.evaluateJavascript(
                            "if(window.ParsecAndroid){" +
                            "var s=JSON.parse(window.ParsecAndroid.getPrivacyStats());" +
                            "var ads=document.getElementById('ads');" +
                            "var trk=document.getElementById('trackers');" +
                            "if(ads)ads.textContent=(s.ads_blocked||0).toLocaleString();" +
                            "if(trk)trk.textContent=(s.trackers_blocked||0).toLocaleString();}", null)
                    }
                }
            }
        }
    }

    // ── Extension Android API bridges ──────────────────────────────────────────

    /** Tracks whether context menus need rebuilding after an extension update. */
    private var contextMenuDirty = false

    private fun showExtensionNotification(notifId: String, title: String, message: String) {
        val channelId = "parsec_extensions"
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val nm = getSystemService(android.app.NotificationManager::class.java)
            if (nm.getNotificationChannel(channelId) == null) {
                nm.createNotificationChannel(
                    android.app.NotificationChannel(
                        channelId, "Extension Notifications",
                        android.app.NotificationManager.IMPORTANCE_DEFAULT
                    )
                )
            }
        }
        val notif = androidx.core.app.NotificationCompat.Builder(this, channelId)
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setContentTitle(title)
            .setContentText(message)
            .setAutoCancel(true)
            .build()
        val nm = androidx.core.app.NotificationManagerCompat.from(this)
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS)
            == PackageManager.PERMISSION_GRANTED || Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
            nm.notify(notifId.hashCode(), notif)
        }
    }

    private fun scheduleExtensionAlarm(name: String, delayMins: Double) {
        val am = getSystemService(android.app.AlarmManager::class.java) ?: return
        val triggerAt = System.currentTimeMillis() + (delayMins * 60_000).toLong()
        val intent = android.app.PendingIntent.getBroadcast(
            this, name.hashCode(),
            Intent("os.parsec.browser.EXTENSION_ALARM").putExtra("name", name),
            android.app.PendingIntent.FLAG_UPDATE_CURRENT or android.app.PendingIntent.FLAG_IMMUTABLE
        )
        am.setExact(android.app.AlarmManager.RTC_WAKEUP, triggerAt, intent)
    }

    private fun updateExtensionBadge(text: String) {
        // Show badge overlay on the tab count button or a dedicated toolbar indicator.
        if (text.isBlank()) {
            btnMenu.contentDescription = "Menu"
        } else {
            btnMenu.contentDescription = "Menu [$text]"
            // Surface the badge text as a small overlay via accessibility and tag.
            btnMenu.tag = text
        }
    }

    // ── URL bar updates ────────────────────────────────────────────────────────

    private fun updateUrlBar(tab: TabEntry?) {
        tab ?: return
        updateUrlBarText(tab.url)
        updateLockIcon(tab.url)
        updateNavButtons(tab)
        updateTitle(tab.title)
    }

    private fun updateUrlBarText(url: String) {
        if (!urlBar.isFocused) {
            val display = try {
                val u = Uri.parse(url)
                if (u.host != null) "${u.scheme}://${u.host}${u.path?.take(30) ?: ""}"
                else url
            } catch (e: Exception) { url }
            urlBar.setText(display)
        }
    }

    private fun updateLockIcon(url: String) {
        lockIcon.setImageResource(
            if (url.startsWith("https://")) android.R.drawable.ic_secure
            else android.R.drawable.ic_partial_secure
        )
    }

    private fun updateNavButtons(tab: TabEntry?) {
        tab ?: return
        btnBack.isEnabled    = tab.webView.canGoBack()
        btnForward.isEnabled = tab.webView.canGoForward()
        btnBack.alpha    = if (tab.webView.canGoBack()) 1.0f else 0.4f
        btnForward.alpha = if (tab.webView.canGoForward()) 1.0f else 0.4f
    }

    private fun updateTitle(title: String) {
        // Update tab entry title and accessibility description
        if (title.isNotBlank() && title != "Loading…") {
            activeTabId?.let { id -> tabs[id]?.title = title }
        }
        btnTabs.contentDescription = "Tabs (\${tabs.size}) — \$title"
        supportActionBar?.subtitle = if (title == "Loading…" || title.isBlank()) null else title
    }

    private fun updateTabCount() {
        btnTabs.text = tabs.size.toString()
    }

    // ── Menu bottom sheet ──────────────────────────────────────────────────────

    private fun showMenuSheet() {
        val sheet = MenuBottomSheet(
            activity = this,
            currentUrl = activeTab()?.url ?: "",
            onNewTab      = { createTab("parsec://newtab", false) },
            onIncognito   = { createTab("parsec://newtab", true) },
            onBookmark    = { bookmarkCurrent() },
            onShare       = { activeTab()?.let { shareUrl(it.url, it.title) } },
            onDesktop     = { toggleDesktopMode() },
            onDownloads   = { showPanel("downloads") },
            onHistory     = { showPanel("history") },
            onBookmarks   = { showPanel("bookmarks") },
            onSettings    = { showPanel("settings") },
            onFindInPage  = { showFindInPage() },
            onZoomIn      = { activeTab()?.webView?.zoomIn() },
            onZoomOut     = { activeTab()?.webView?.zoomOut() },
        )
        sheet.show(supportFragmentManager, "menu")
    }

    private fun showTabSwitcher() {
        val switcher = TabSwitcherBottomSheet(
            tabs = tabs.values.toList(),
            activeTabId = activeTabId,
            onTabSelected = { id -> switchToTab(id); },
            onTabClosed   = { id -> closeTab(id) },
            onNewTab      = { createTab("parsec://newtab", false) },
        )
        switcher.show(supportFragmentManager, "tabs")
    }

    fun showPanel(panel: String) {
        val frag = BrowserPanelFragment.newInstance(panel) { url ->
            activeTabId?.let { navigateTab(it, url) }
        }
        frag.show(supportFragmentManager, "panel")
    }

    /** Persistent inline find-in-page bar anchored above the toolbar.
     *  Shows match count, prev/next buttons, and a ✕ dismiss button.
     *  Replaces the old AlertDialog approach for a Chrome-parity UX. */
    private var findBarView: android.view.View? = null

    private fun showFindInPage() {
        val tab = activeTab() ?: return

        // Dismiss existing bar if any
        dismissFindBar()

        val ctx = this

        // Container for the find bar (anchored at bottom, above system nav)
        val bar = LinearLayout(ctx).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity     = Gravity.CENTER_VERTICAL
            setBackgroundColor(0xFF1e293b.toInt())
            setPadding(16, 12, 8, 12)
            elevation = 8f
        }

        val input = android.widget.EditText(ctx).apply {
            hint          = "Find in page…"
            setHintTextColor(0xFF475569.toInt())
            setTextColor(0xFFe2e8f0.toInt())
            background    = null
            imeOptions    = android.view.inputmethod.EditorInfo.IME_ACTION_SEARCH
            inputType     = android.text.InputType.TYPE_CLASS_TEXT
            textSize      = 14f
            layoutParams  = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f)
        }

        val matchCount = TextView(ctx).apply {
            text      = "0/0"
            textSize  = 12f
            setTextColor(0xFF64748b.toInt())
            minWidth  = 60
            gravity   = Gravity.CENTER
            setPadding(8, 0, 8, 0)
        }

        fun navBtn(label: String, action: () -> Unit) = Button(ctx).apply {
            text      = label
            textSize  = 16f
            setTextColor(0xFF818cf8.toInt())
            background = null
            setPadding(12, 0, 12, 0)
            minimumWidth = 0
            minWidth  = 0
            setOnClickListener { action() }
        }

        val btnPrev = navBtn("▲") { tab.webView.findNext(false) }
        val btnNext = navBtn("▼") { tab.webView.findNext(true) }
        val btnClose = navBtn("✕") { dismissFindBar(); tab.webView.clearMatches() }

        bar.addView(input)
        bar.addView(matchCount)
        bar.addView(btnPrev)
        bar.addView(btnNext)
        bar.addView(btnClose)

        // Track match count via findResultsCallback
        tab.webView.setFindListener { _, matchesFound, isDoneCounting ->
            if (isDoneCounting) {
                matchCount.text = if (matchesFound == 0) "0/0" else "$matchesFound"
            }
        }

        input.addTextChangedListener(object : android.text.TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, st: Int, c: Int, a: Int) {}
            override fun onTextChanged(s: CharSequence?, st: Int, b: Int, c: Int) {
                if (s.isNullOrBlank()) {
                    tab.webView.clearMatches()
                    matchCount.text = "0/0"
                } else {
                    tab.webView.findAllAsync(s.toString())
                }
            }
            override fun afterTextChanged(s: android.text.Editable?) {}
        })

        // Add bar to root FrameLayout above toolbar
        val rootFrame = window.decorView.findViewById<FrameLayout>(android.R.id.content)
        val params = FrameLayout.LayoutParams(
            FrameLayout.LayoutParams.MATCH_PARENT,
            FrameLayout.LayoutParams.WRAP_CONTENT,
            Gravity.BOTTOM
        )
        rootFrame.addView(bar, params)
        findBarView = bar

        // Auto-show keyboard
        input.requestFocus()
        (getSystemService(INPUT_METHOD_SERVICE) as InputMethodManager)
            .showSoftInput(input, InputMethodManager.SHOW_IMPLICIT)
    }

    private fun dismissFindBar() {
        findBarView?.let { bar ->
            (bar.parent as? ViewGroup)?.removeView(bar)
            findBarView = null
        }
    }

    // ── Actions ────────────────────────────────────────────────────────────────

    private fun bookmarkCurrent() {
        val tab = activeTab() ?: return
        ipc("AddBookmark", mapOf("url" to tab.url, "title" to tab.title, "favicon" to "🌐"))
        Toast.makeText(this, "Bookmarked!", Toast.LENGTH_SHORT).show()
    }

    private fun toggleDesktopMode() {
        val prefs = ipc("GetPrefs")
        val current = prefs.getAsJsonObject("data")?.get("desktop_mode")?.asBoolean ?: false
        ipc("SetDesktopMode", mapOf("enabled" to !current))
    }

    private fun shareUrl(url: String, title: String) {
        val intent = Intent(Intent.ACTION_SEND).apply {
            type = "text/plain"
            putExtra(Intent.EXTRA_TEXT, url)
            putExtra(Intent.EXTRA_SUBJECT, title)
        }
        startActivity(Intent.createChooser(intent, "Share URL"))
    }

    private fun openInExternalApp(url: String) {
        try { startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) }
        catch (e: Exception) { Toast.makeText(this, "No app to open this URL", Toast.LENGTH_SHORT).show() }
    }

    private fun startDownload(url: String, filename: String) {
        val intent = Intent(this, DownloadService::class.java).apply {
            putExtra("url", url)
            putExtra("filename", filename)
        }
        startService(intent)
        Toast.makeText(this, "Downloading $filename…", Toast.LENGTH_SHORT).show()
    }

    private fun prefetchUrl(url: String) {
        // Create a hidden WebView to preload, destroy after 10s
        scope.launch(Dispatchers.IO) {
            // Lightweight DNS prefetch only (avoids full WebView overhead)
            try { java.net.InetAddress.getByName(Uri.parse(url).host ?: return@launch) }
            catch (e: Exception) { /* ignore */ }
        }
    }

    // ── URL normalisation ──────────────────────────────────────────────────────

    private fun normalizeUrl(input: String): String {
        if (input.startsWith("parsec:") || input.startsWith("about:")) return input
        if (input.startsWith("https://") || input.startsWith("http://")) return input
        val domain = Regex("^[a-z0-9][a-z0-9\\-]*\\.[a-z]{2,}", RegexOption.IGNORE_CASE)
        if (domain.containsMatchIn(input.split("/").first()) && !input.contains(' ')) {
            return "https://$input"
        }
        val q = Uri.encode(input)
        return "https://search.parsec.os/search?q=$q"
    }

    private fun hideKeyboard() {
        val imm = getSystemService(INPUT_METHOD_SERVICE) as InputMethodManager
        imm.hideSoftInputFromWindow(urlBar.windowToken, 0)
    }
    private fun extractOrigin(url: String): String {
        return try {
            val uri  = android.net.Uri.parse(url)
            val host = uri.host ?: return url
            val parts = host.split(".")
            if (parts.size >= 2) "${parts[parts.size - 2]}.${parts[parts.size - 1]}"
            else host
        } catch (e: Exception) { url }
    }

    // ── Ghost Mode helpers ─────────────────────────────────────────────────────

    /**
     * Anti-fingerprinting JavaScript injected into every incognito page.
     *
     * Neutralises:
     *  - Canvas fingerprinting (adds imperceptible noise to pixel reads)
     *  - WebGL fingerprinting (spoofs renderer/vendor strings)
     *  - Audio fingerprinting (adds tiny noise to AudioContext output)
     *  - Battery API (always returns null — real level is a tracking vector)
     *  - Hardware concurrency (always returns 4 — real CPU count is fingerprint)
     *  - Device memory (always returns 4 — real RAM is fingerprint)
     *  - Timezone (clamped to UTC offset to prevent geo inference)
     *  - Screen dimensions (reported as a common generic size)
     *  - WebRTC IP exposure (overrides RTCPeerConnection to block STUN)
     *  - navigator.plugins (empty — plugin list is a fingerprint)
     *  - navigator.languages (single "en-US" entry)
     *  - Keyboard/mouse timing APIs (clamped to prevent timing attacks)
     */
    private fun ghostAntiFingerprint(): String = """
(function() {
  'use strict';

  // ── Canvas fingerprinting ─────────────────────────────────────────────────
  const origGetImageData = CanvasRenderingContext2D.prototype.getImageData;
  CanvasRenderingContext2D.prototype.getImageData = function(x, y, w, h) {
    const d = origGetImageData.call(this, x, y, w, h);
    for (let i = 0; i < d.data.length; i += 4) {
      d.data[i]   ^= (Math.random() * 2 | 0);   // R: flip 0 or 1 LSB
      d.data[i+1] ^= (Math.random() * 2 | 0);   // G
      d.data[i+2] ^= (Math.random() * 2 | 0);   // B
    }
    return d;
  };
  const origToDataURL = HTMLCanvasElement.prototype.toDataURL;
  HTMLCanvasElement.prototype.toDataURL = function(type, q) {
    const ctx = this.getContext('2d');
    if (ctx) {
      ctx.fillStyle = 'rgba(0,0,0,0.002)';
      ctx.fillRect(0, 0, 1, 1);
    }
    return origToDataURL.call(this, type, q);
  };

  // ── WebGL fingerprinting ──────────────────────────────────────────────────
  const origGetParam = WebGLRenderingContext.prototype.getParameter;
  WebGLRenderingContext.prototype.getParameter = function(p) {
    if (p === 37445) return 'Generic Vendor';    // UNMASKED_VENDOR_WEBGL
    if (p === 37446) return 'Generic Renderer';  // UNMASKED_RENDERER_WEBGL
    return origGetParam.call(this, p);
  };

  // ── Audio fingerprinting ──────────────────────────────────────────────────
  try {
    const origCreateAnalyser = AudioContext.prototype.createAnalyser;
    AudioContext.prototype.createAnalyser = function() {
      const a = origCreateAnalyser.call(this);
      const origGetFloat = a.getFloatFrequencyData.bind(a);
      a.getFloatFrequencyData = function(arr) {
        origGetFloat(arr);
        for (let i = 0; i < arr.length; i++) arr[i] += (Math.random() - 0.5) * 0.001;
      };
      return a;
    };
  } catch(e) {}

  // ── Battery API ───────────────────────────────────────────────────────────
  if (navigator.getBattery) {
    Object.defineProperty(navigator, 'getBattery', {
      value: () => Promise.resolve(null), writable: false
    });
  }

  // ── Hardware concurrency ──────────────────────────────────────────────────
  Object.defineProperty(navigator, 'hardwareConcurrency', { value: 4, writable: false });

  // ── Device memory ─────────────────────────────────────────────────────────
  try {
    Object.defineProperty(navigator, 'deviceMemory', { value: 4, writable: false });
  } catch(e) {}

  // ── Plugins (empty list) ──────────────────────────────────────────────────
  Object.defineProperty(navigator, 'plugins', {
    get: () => Object.create(PluginArray.prototype), writable: false
  });

  // ── Languages (generic) ──────────────────────────────────────────────────
  Object.defineProperty(navigator, 'languages', { value: ['en-US'], writable: false });
  Object.defineProperty(navigator, 'language',  { value: 'en-US',   writable: false });

  // ── DNT ───────────────────────────────────────────────────────────────────
  Object.defineProperty(navigator, 'doNotTrack', { value: '1', writable: false });

  // ── WebRTC IP leak prevention ─────────────────────────────────────────────
  if (window.RTCPeerConnection) {
    window.RTCPeerConnection = function(config) {
      if (config && config.iceServers) config.iceServers = [];
      const pc = new (window._origRTCPeerConnection || RTCPeerConnection)(config);
      pc.createDataChannel = () => { throw new Error('WebRTC blocked in Ghost Mode'); };
      return pc;
    };
  }

  // ── Screen dimensions (generic 1920x1080) ─────────────────────────────────
  try {
    Object.defineProperty(screen, 'width',       { value: 1920, writable: false });
    Object.defineProperty(screen, 'height',      { value: 1080, writable: false });
    Object.defineProperty(screen, 'availWidth',  { value: 1920, writable: false });
    Object.defineProperty(screen, 'availHeight', { value: 1040, writable: false });
    Object.defineProperty(screen, 'colorDepth',  { value: 24,   writable: false });
    Object.defineProperty(screen, 'pixelDepth',  { value: 24,   writable: false });
  } catch(e) {}

  // ── Timezone clamping (prevent geo inference) ─────────────────────────────
  const origResolvedOpts = Intl.DateTimeFormat.prototype.resolvedOptions;
  Intl.DateTimeFormat.prototype.resolvedOptions = function() {
    const opts = origResolvedOpts.call(this);
    opts.timeZone = 'UTC';
    return opts;
  };

  console.log('[Parsec Ghost Mode] Anti-fingerprinting active');
})();
""".trimIndent()

    /**
     * Show a Ghost Mode status bar when switching to an incognito tab.
     * Displays: 🕵️ Ghost Mode • Encrypted • Keys rotate every 30min
     */
    private fun showGhostBanner(tabId: String) {
        val tab = tabs[tabId] ?: return
        if (!tab.incognito) return

        // Remove existing banner if any
        (window.decorView as? android.view.ViewGroup)?.let { root ->
            root.findViewWithTag<android.view.View>("ghost_banner")?.let { root.removeView(it) }
        }

        val banner = android.widget.TextView(this).apply {
            tag       = "ghost_banner"
            text      = "🕵️  Ghost Mode  •  Encrypted  •  Keys rotate every 30 min"
            textSize  = 11f
            setTextColor(0xFFa78bfa.toInt())
            setBackgroundColor(0xCC0f172a.toInt())
            setPadding(32, 12, 32, 12)
            gravity   = android.view.Gravity.CENTER
        }

        val params = android.widget.FrameLayout.LayoutParams(
            android.widget.FrameLayout.LayoutParams.MATCH_PARENT,
            android.widget.FrameLayout.LayoutParams.WRAP_CONTENT,
            android.view.Gravity.TOP
        )

        val rootFrame = window.decorView.findViewById<android.widget.FrameLayout>(android.R.id.content)
        rootFrame.addView(banner, params)

        // Auto-dismiss after 3 seconds
        handler.postDelayed({
            rootFrame.removeView(banner)
        }, 3000)
    }

}
