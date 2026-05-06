package os.parsec.browser

/**
 * JNI bridge to the Rust parsec_core library.
 * Every method maps 1:1 to an extern "system" fn in src-rust/src/lib.rs.
 *
 * All methods are declared inside a single Kotlin object so there is exactly
 * one class file (ParsecCore.class) and the JNI name mangling is predictable:
 *   Java_os_parsec_browser_ParsecCore_<methodName>
 */
object ParsecCore {

    // ── Core lifecycle ─────────────────────────────────────────────────────────

    /** Initialise Rust core. Call once from Application.onCreate(). */
    external fun init(dataDir: String)

    /** App going to background — pause GPU, persist prefs. */
    external fun onPause()

    /** App returning to foreground — resume GPU. */
    external fun onResume()

    /** Full shutdown — persist everything and release GPU. */
    external fun shutdown()

    // ── IPC ────────────────────────────────────────────────────────────────────

    /**
     * Main IPC dispatcher. Send a JSON command, get a JSON response.
     * @param json  e.g. {"id":"1","cmd":"GetPrefs","args":{}}
     * @return      e.g. {"id":"1","ok":true,"data":{...}}
     */
    external fun ipc(json: String): String

    /**
     * Poll pending events from the Rust side.
     * Returns a JSON array. Call on a ~16ms timer.
     */
    external fun pollEvents(): String

    // ── Navigation ────────────────────────────────────────────────────────────

    /**
     * Called before each WebView navigation.
     * Returns: { "allow": bool, "redirect_url": string|null, "reason": string|null }
     */
    external fun shouldAllowNavigation(tabId: String, url: String): String

    /**
     * Called for every subresource request (WebViewClient.shouldInterceptRequest).
     * Returns: { "block": bool, "reason": string|null }
     */
    external fun shouldBlockResource(tabId: String, url: String, resourceType: String): String

    // ── Tab lifecycle ─────────────────────────────────────────────────────────

    /** Notify Rust when a WebView's URL/title/nav state changes. */
    external fun onTabUpdated(
        tabId:   String,
        url:     String,
        title:   String,
        canBack: Boolean,
        canFwd:  Boolean,
        loading: Boolean
    )

    /** Notify Rust of a favicon change. */
    external fun onFaviconChanged(tabId: String, faviconUrl: String)

    // ── Address bar ───────────────────────────────────────────────────────────

    /** Get address bar autocomplete suggestions. Returns a JSON array. */
    external fun getSuggestions(query: String): String

    // ── Neutron GPU compositor ────────────────────────────────────────────────

    /** Initialise Neutron GPU compositor on an ANativeWindow*. */
    external fun neutronInit(surfacePtr: Long, width: Int, height: Int): Boolean

    /** Render one frame. Call from Choreographer callback. */
    external fun neutronFrame()

    /** Notify Neutron of surface resize. */
    external fun neutronResize(width: Int, height: Int)

    // ── Ghost Mode (encrypted incognito) ──────────────────────────────────────

    /**
     * Create a Ghost Mode session for an incognito tab.
     * Generates fresh ephemeral ChaCha20 keys + randomised user-agent.
     * Call immediately after createTab() when incognito=true.
     */
    external fun ghostCreateSession(tabId: String)

    /**
     * Destroy a Ghost Mode session.
     * Zeroes all key material in memory immediately.
     * Call when an incognito tab is closed.
     */
    external fun ghostDestroySession(tabId: String)

    /**
     * Get the randomised desktop user-agent for a ghost tab.
     * Returns a different UA string per session; rotates every 30 min.
     */
    external fun ghostGetUserAgent(tabId: String): String

    /**
     * Get Ghost Mode status as JSON:
     * { "enabled": bool, "session_count": int, "has_proxy_server": bool,
     *   "hop_count": int, "dns_private": bool }
     */
    external fun ghostGetStatus(): String

    /**
     * Configure phantom proxy servers.
     * @param configJson JSON: { "entry_node": "wss://...", "exit_node": "wss://...",
     *                           "middle_node": null, "private_dns": true }
     */
    external fun ghostConfigure(configJson: String)
}
