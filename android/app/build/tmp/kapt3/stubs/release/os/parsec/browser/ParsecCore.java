package os.parsec.browser;

/**
 * JNI bridge to the Rust parsec_core library.
 * Every method maps 1:1 to an extern "system" fn in src-rust/src/lib.rs.
 *
 * All methods are declared inside a single Kotlin object so there is exactly
 * one class file (ParsecCore.class) and the JNI name mangling is predictable:
 *  Java_os_parsec_browser_ParsecCore_<methodName>
 */
@kotlin.Metadata(mv = {1, 9, 0}, k = 1, xi = 48, d1 = {"\u00000\n\u0002\u0018\u0002\n\u0002\u0010\u0000\n\u0002\b\u0002\n\u0002\u0010\u000e\n\u0002\b\u0002\n\u0002\u0010\u0002\n\u0002\b\f\n\u0002\u0010\u000b\n\u0000\n\u0002\u0010\t\n\u0000\n\u0002\u0010\b\n\u0002\b\u0012\b\u00c6\u0002\u0018\u00002\u00020\u0001B\u0007\b\u0002\u00a2\u0006\u0002\u0010\u0002J\u0011\u0010\u0003\u001a\u00020\u00042\u0006\u0010\u0005\u001a\u00020\u0004H\u0086 J\u0011\u0010\u0006\u001a\u00020\u00072\u0006\u0010\b\u001a\u00020\u0004H\u0086 J\u0011\u0010\t\u001a\u00020\u00072\u0006\u0010\n\u001a\u00020\u0004H\u0086 J\u0011\u0010\u000b\u001a\u00020\u00072\u0006\u0010\n\u001a\u00020\u0004H\u0086 J\t\u0010\f\u001a\u00020\u0004H\u0086 J\u0011\u0010\r\u001a\u00020\u00042\u0006\u0010\n\u001a\u00020\u0004H\u0086 J\u0011\u0010\u000e\u001a\u00020\u00072\u0006\u0010\u000f\u001a\u00020\u0004H\u0086 J\u0011\u0010\u0010\u001a\u00020\u00042\u0006\u0010\u0011\u001a\u00020\u0004H\u0086 J\t\u0010\u0012\u001a\u00020\u0007H\u0086 J!\u0010\u0013\u001a\u00020\u00142\u0006\u0010\u0015\u001a\u00020\u00162\u0006\u0010\u0017\u001a\u00020\u00182\u0006\u0010\u0019\u001a\u00020\u0018H\u0086 J\u0019\u0010\u001a\u001a\u00020\u00072\u0006\u0010\u0017\u001a\u00020\u00182\u0006\u0010\u0019\u001a\u00020\u0018H\u0086 J\u0019\u0010\u001b\u001a\u00020\u00072\u0006\u0010\n\u001a\u00020\u00042\u0006\u0010\u001c\u001a\u00020\u0004H\u0086 J\t\u0010\u001d\u001a\u00020\u0007H\u0086 J\t\u0010\u001e\u001a\u00020\u0007H\u0086 J9\u0010\u001f\u001a\u00020\u00072\u0006\u0010\n\u001a\u00020\u00042\u0006\u0010 \u001a\u00020\u00042\u0006\u0010!\u001a\u00020\u00042\u0006\u0010\"\u001a\u00020\u00142\u0006\u0010#\u001a\u00020\u00142\u0006\u0010$\u001a\u00020\u0014H\u0086 J\t\u0010%\u001a\u00020\u0004H\u0086 J\u0019\u0010&\u001a\u00020\u00042\u0006\u0010\n\u001a\u00020\u00042\u0006\u0010 \u001a\u00020\u0004H\u0086 J!\u0010\'\u001a\u00020\u00042\u0006\u0010\n\u001a\u00020\u00042\u0006\u0010 \u001a\u00020\u00042\u0006\u0010(\u001a\u00020\u0004H\u0086 J\t\u0010)\u001a\u00020\u0007H\u0086 \u00a8\u0006*"}, d2 = {"Los/parsec/browser/ParsecCore;", "", "()V", "getSuggestions", "", "query", "ghostConfigure", "", "configJson", "ghostCreateSession", "tabId", "ghostDestroySession", "ghostGetStatus", "ghostGetUserAgent", "init", "dataDir", "ipc", "json", "neutronFrame", "neutronInit", "", "surfacePtr", "", "width", "", "height", "neutronResize", "onFaviconChanged", "faviconUrl", "onPause", "onResume", "onTabUpdated", "url", "title", "canBack", "canFwd", "loading", "pollEvents", "shouldAllowNavigation", "shouldBlockResource", "resourceType", "shutdown", "app_release"})
public final class ParsecCore {
    @org.jetbrains.annotations.NotNull()
    public static final os.parsec.browser.ParsecCore INSTANCE = null;
    
    private ParsecCore() {
        super();
    }
    
    /**
     * Initialise Rust core. Call once from Application.onCreate().
     */
    public final native void init(@org.jetbrains.annotations.NotNull()
    java.lang.String dataDir) {
    }
    
    /**
     * App going to background — pause GPU, persist prefs.
     */
    public final native void onPause() {
    }
    
    /**
     * App returning to foreground — resume GPU.
     */
    public final native void onResume() {
    }
    
    /**
     * Full shutdown — persist everything and release GPU.
     */
    public final native void shutdown() {
    }
    
    /**
     * Main IPC dispatcher. Send a JSON command, get a JSON response.
     * @param json  e.g. {"id":"1","cmd":"GetPrefs","args":{}}
     * @return      e.g. {"id":"1","ok":true,"data":{...}}
     */
    @org.jetbrains.annotations.NotNull()
    public final native java.lang.String ipc(@org.jetbrains.annotations.NotNull()
    java.lang.String json) {
        return null;
    }
    
    /**
     * Poll pending events from the Rust side.
     * Returns a JSON array. Call on a ~16ms timer.
     */
    @org.jetbrains.annotations.NotNull()
    public final native java.lang.String pollEvents() {
        return null;
    }
    
    /**
     * Called before each WebView navigation.
     * Returns: { "allow": bool, "redirect_url": string|null, "reason": string|null }
     */
    @org.jetbrains.annotations.NotNull()
    public final native java.lang.String shouldAllowNavigation(@org.jetbrains.annotations.NotNull()
    java.lang.String tabId, @org.jetbrains.annotations.NotNull()
    java.lang.String url) {
        return null;
    }
    
    /**
     * Called for every subresource request (WebViewClient.shouldInterceptRequest).
     * Returns: { "block": bool, "reason": string|null }
     */
    @org.jetbrains.annotations.NotNull()
    public final native java.lang.String shouldBlockResource(@org.jetbrains.annotations.NotNull()
    java.lang.String tabId, @org.jetbrains.annotations.NotNull()
    java.lang.String url, @org.jetbrains.annotations.NotNull()
    java.lang.String resourceType) {
        return null;
    }
    
    /**
     * Notify Rust when a WebView's URL/title/nav state changes.
     */
    public final native void onTabUpdated(@org.jetbrains.annotations.NotNull()
    java.lang.String tabId, @org.jetbrains.annotations.NotNull()
    java.lang.String url, @org.jetbrains.annotations.NotNull()
    java.lang.String title, boolean canBack, boolean canFwd, boolean loading) {
    }
    
    /**
     * Notify Rust of a favicon change.
     */
    public final native void onFaviconChanged(@org.jetbrains.annotations.NotNull()
    java.lang.String tabId, @org.jetbrains.annotations.NotNull()
    java.lang.String faviconUrl) {
    }
    
    /**
     * Get address bar autocomplete suggestions. Returns a JSON array.
     */
    @org.jetbrains.annotations.NotNull()
    public final native java.lang.String getSuggestions(@org.jetbrains.annotations.NotNull()
    java.lang.String query) {
        return null;
    }
    
    /**
     * Initialise Neutron GPU compositor on an ANativeWindow*.
     */
    public final native boolean neutronInit(long surfacePtr, int width, int height) {
        return false;
    }
    
    /**
     * Render one frame. Call from Choreographer callback.
     */
    public final native void neutronFrame() {
    }
    
    /**
     * Notify Neutron of surface resize.
     */
    public final native void neutronResize(int width, int height) {
    }
    
    /**
     * Create a Ghost Mode session for an incognito tab.
     * Generates fresh ephemeral ChaCha20 keys + randomised user-agent.
     * Call immediately after createTab() when incognito=true.
     */
    public final native void ghostCreateSession(@org.jetbrains.annotations.NotNull()
    java.lang.String tabId) {
    }
    
    /**
     * Destroy a Ghost Mode session.
     * Zeroes all key material in memory immediately.
     * Call when an incognito tab is closed.
     */
    public final native void ghostDestroySession(@org.jetbrains.annotations.NotNull()
    java.lang.String tabId) {
    }
    
    /**
     * Get the randomised desktop user-agent for a ghost tab.
     * Returns a different UA string per session; rotates every 30 min.
     */
    @org.jetbrains.annotations.NotNull()
    public final native java.lang.String ghostGetUserAgent(@org.jetbrains.annotations.NotNull()
    java.lang.String tabId) {
        return null;
    }
    
    /**
     * Get Ghost Mode status as JSON:
     * { "enabled": bool, "session_count": int, "has_proxy_server": bool,
     *  "hop_count": int, "dns_private": bool }
     */
    @org.jetbrains.annotations.NotNull()
    public final native java.lang.String ghostGetStatus() {
        return null;
    }
    
    /**
     * Configure phantom proxy servers.
     * @param configJson JSON: { "entry_node": "wss://...", "exit_node": "wss://...",
     *                          "middle_node": null, "private_dns": true }
     */
    public final native void ghostConfigure(@org.jetbrains.annotations.NotNull()
    java.lang.String configJson) {
    }
}