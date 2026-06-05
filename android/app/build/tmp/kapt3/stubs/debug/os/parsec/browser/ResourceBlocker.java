package os.parsec.browser;

/**
 * Lightweight navigation helper — subresource intercept is DISABLED globally
 * because shouldInterceptRequest on WebView causes severe jank on all modern sites.
 */
@kotlin.Metadata(mv = {1, 9, 0}, k = 1, xi = 48, d1 = {"\u0000d\n\u0002\u0018\u0002\n\u0002\u0010\u0000\n\u0002\b\u0002\n\u0002\u0010\u000b\n\u0000\n\u0002\u0010\u000e\n\u0000\n\u0002\u0018\u0002\n\u0002\u0018\u0002\n\u0002\b\f\n\u0002\u0018\u0002\n\u0002\b\u0005\n\u0002\u0010\u0011\n\u0002\b\u0004\n\u0002\u0018\u0002\n\u0002\b\u0003\n\u0002\u0018\u0002\n\u0002\b\u0003\n\u0002\u0018\u0002\n\u0002\b\u0006\n\u0002\u0010\u0002\n\u0002\b\u0003\n\u0002\u0010\"\n\u0002\b\u0006\n\u0002\u0018\u0002\n\u0002\b\u0003\b\u00c6\u0002\u0018\u00002\u00020\u0001:\u0002<=B\u0007\b\u0002\u00a2\u0006\u0002\u0010\u0002J\u0006\u0010 \u001a\u00020!J\u0018\u0010\"\u001a\u00020\u00062\u0006\u0010#\u001a\u00020\u00062\b\b\u0002\u0010$\u001a\u00020%J\u0012\u0010&\u001a\u0004\u0018\u00010\u00062\u0006\u0010\'\u001a\u00020\u0006H\u0002J\u000e\u0010(\u001a\u00020)2\u0006\u0010*\u001a\u00020\u0006J\u001a\u0010+\u001a\u0004\u0018\u00010\u00062\b\u0010,\u001a\u0004\u0018\u00010\u00062\u0006\u0010-\u001a\u00020\u0006J\u0012\u0010.\u001a\u0004\u0018\u00010\u00062\u0006\u0010*\u001a\u00020\u0006H\u0002J\b\u0010/\u001a\u000200H\u0002J\u0006\u00101\u001a\u000200J\u001e\u00102\u001a\u00020\u00042\u0006\u0010\'\u001a\u00020\u00062\f\u00103\u001a\b\u0012\u0004\u0012\u00020\u000604H\u0002J\u000e\u00105\u001a\u00020\u00042\u0006\u0010\'\u001a\u00020\u0006J\b\u00106\u001a\u000200H\u0002J\u0010\u00107\u001a\u0002002\u0006\u00108\u001a\u00020\u0006H\u0002J\u000e\u00109\u001a\u0002002\u0006\u0010:\u001a\u00020;R\u000e\u0010\u0003\u001a\u00020\u0004X\u0086T\u00a2\u0006\u0002\n\u0000R\u000e\u0010\u0005\u001a\u00020\u0006X\u0082T\u00a2\u0006\u0002\n\u0000R\u001e\u0010\u0007\u001a\u0012\u0012\u0004\u0012\u00020\u00060\bj\b\u0012\u0004\u0012\u00020\u0006`\tX\u0082\u000e\u00a2\u0006\u0002\n\u0000R\u001a\u0010\n\u001a\u00020\u0004X\u0086\u000e\u00a2\u0006\u000e\n\u0000\u001a\u0004\b\u000b\u0010\f\"\u0004\b\r\u0010\u000eR\u001a\u0010\u000f\u001a\u00020\u0004X\u0086\u000e\u00a2\u0006\u000e\n\u0000\u001a\u0004\b\u0010\u0010\f\"\u0004\b\u0011\u0010\u000eR\u001a\u0010\u0012\u001a\u00020\u0004X\u0086\u000e\u00a2\u0006\u000e\n\u0000\u001a\u0004\b\u0013\u0010\f\"\u0004\b\u0014\u0010\u000eR\u001a\u0010\u0015\u001a\u000e\u0012\u0004\u0012\u00020\u0006\u0012\u0004\u0012\u00020\u00040\u0016X\u0082\u0004\u00a2\u0006\u0002\n\u0000R\u001a\u0010\u0017\u001a\u00020\u0004X\u0086\u000e\u00a2\u0006\u000e\n\u0000\u001a\u0004\b\u0018\u0010\f\"\u0004\b\u0019\u0010\u000eR\u000e\u0010\u001a\u001a\u00020\u0004X\u0082\u000e\u00a2\u0006\u0002\n\u0000R\u0016\u0010\u001b\u001a\b\u0012\u0004\u0012\u00020\u00060\u001cX\u0082\u0004\u00a2\u0006\u0004\n\u0002\u0010\u001dR\u0016\u0010\u001e\u001a\b\u0012\u0004\u0012\u00020\u00060\u001cX\u0082\u0004\u00a2\u0006\u0004\n\u0002\u0010\u001dR\u001e\u0010\u001f\u001a\u0012\u0012\u0004\u0012\u00020\u00060\bj\b\u0012\u0004\u0012\u00020\u0006`\tX\u0082\u000e\u00a2\u0006\u0002\n\u0000\u00a8\u0006>"}, d2 = {"Los/parsec/browser/ResourceBlocker;", "", "()V", "SUBRESOURCE_BLOCKING_ENABLED", "", "TAG", "", "adHosts", "Ljava/util/HashSet;", "Lkotlin/collections/HashSet;", "blockAds", "getBlockAds", "()Z", "setBlockAds", "(Z)V", "blockNsfw", "getBlockNsfw", "setBlockNsfw", "blockTrackers", "getBlockTrackers", "setBlockTrackers", "hostCache", "Landroidx/collection/LruCache;", "httpsOnly", "getHttpsOnly", "setHttpsOnly", "initialized", "minerKeywords", "", "[Ljava/lang/String;", "nsfwKeywords", "trackerHosts", "blockedResponse", "Landroid/webkit/WebResourceResponse;", "buildSearchUrl", "query", "engine", "Los/parsec/browser/ResourceBlocker$SearchEngine;", "checkHost", "host", "checkNavigation", "Los/parsec/browser/ResourceBlocker$NavDecision;", "url", "checkSubresource", "pageUrl", "requestUrl", "extractHost", "initFallback", "", "initFromRust", "isBlockedHost", "blocked", "", "isTrustedHost", "loadPrefsSafe", "parseBlockLists", "json", "refreshPrefs", "prefsJson", "Lcom/google/gson/JsonObject;", "NavDecision", "SearchEngine", "app_debug"})
public final class ResourceBlocker {
    @org.jetbrains.annotations.NotNull()
    private static final java.lang.String TAG = "ResourceBlocker";
    
    /**
     * Subresource blocking is off — it breaks video sites and causes universal lag.
     */
    public static final boolean SUBRESOURCE_BLOCKING_ENABLED = false;
    @kotlin.jvm.Volatile()
    private static volatile boolean blockAds = false;
    @kotlin.jvm.Volatile()
    private static volatile boolean blockTrackers = false;
    @kotlin.jvm.Volatile()
    private static volatile boolean blockNsfw = false;
    @kotlin.jvm.Volatile()
    private static volatile boolean httpsOnly = true;
    @org.jetbrains.annotations.NotNull()
    private static java.util.HashSet<java.lang.String> adHosts;
    @org.jetbrains.annotations.NotNull()
    private static java.util.HashSet<java.lang.String> trackerHosts;
    @org.jetbrains.annotations.NotNull()
    private static final androidx.collection.LruCache<java.lang.String, java.lang.Boolean> hostCache = null;
    @kotlin.jvm.Volatile()
    private static volatile boolean initialized = false;
    @org.jetbrains.annotations.NotNull()
    private static final java.lang.String[] nsfwKeywords = {"pornhub", "xvideos", "xnxx", "redtube", "youporn"};
    @org.jetbrains.annotations.NotNull()
    private static final java.lang.String[] minerKeywords = {"coinhive", "cryptoloot", "coin-hive", "minero.cc", "webmr.ru"};
    @org.jetbrains.annotations.NotNull()
    public static final os.parsec.browser.ResourceBlocker INSTANCE = null;
    
    private ResourceBlocker() {
        super();
    }
    
    public final boolean getBlockAds() {
        return false;
    }
    
    public final void setBlockAds(boolean p0) {
    }
    
    public final boolean getBlockTrackers() {
        return false;
    }
    
    public final void setBlockTrackers(boolean p0) {
    }
    
    public final boolean getBlockNsfw() {
        return false;
    }
    
    public final void setBlockNsfw(boolean p0) {
    }
    
    public final boolean getHttpsOnly() {
        return false;
    }
    
    public final void setHttpsOnly(boolean p0) {
    }
    
    public final void initFromRust() {
    }
    
    private final void parseBlockLists(java.lang.String json) {
    }
    
    private final void initFallback() {
    }
    
    private final void loadPrefsSafe() {
    }
    
    public final void refreshPrefs(@org.jetbrains.annotations.NotNull()
    com.google.gson.JsonObject prefsJson) {
    }
    
    /**
     * Navigation: HTTPS upgrade only — never block main-frame loads (breaks sites).
     */
    @org.jetbrains.annotations.NotNull()
    public final os.parsec.browser.ResourceBlocker.NavDecision checkNavigation(@org.jetbrains.annotations.NotNull()
    java.lang.String url) {
        return null;
    }
    
    @org.jetbrains.annotations.NotNull()
    public final java.lang.String buildSearchUrl(@org.jetbrains.annotations.NotNull()
    java.lang.String query, @org.jetbrains.annotations.NotNull()
    os.parsec.browser.ResourceBlocker.SearchEngine engine) {
        return null;
    }
    
    @org.jetbrains.annotations.NotNull()
    public final android.webkit.WebResourceResponse blockedResponse() {
        return null;
    }
    
    /**
     * Not used while SUBRESOURCE_BLOCKING_ENABLED is false.
     */
    @org.jetbrains.annotations.Nullable()
    public final java.lang.String checkSubresource(@org.jetbrains.annotations.Nullable()
    java.lang.String pageUrl, @org.jetbrains.annotations.NotNull()
    java.lang.String requestUrl) {
        return null;
    }
    
    public final boolean isTrustedHost(@kotlin.Suppress(names = {"UNUSED_PARAMETER"})
    @org.jetbrains.annotations.NotNull()
    java.lang.String host) {
        return false;
    }
    
    private final java.lang.String checkHost(java.lang.String host) {
        return null;
    }
    
    private final boolean isBlockedHost(java.lang.String host, java.util.Set<java.lang.String> blocked) {
        return false;
    }
    
    private final java.lang.String extractHost(java.lang.String url) {
        return null;
    }
    
    @kotlin.Metadata(mv = {1, 9, 0}, k = 1, xi = 48, d1 = {"\u0000 \n\u0002\u0018\u0002\n\u0002\u0010\u0000\n\u0000\n\u0002\u0010\u000b\n\u0000\n\u0002\u0010\u000e\n\u0002\b\u000e\n\u0002\u0010\b\n\u0002\b\u0002\b\u0086\b\u0018\u00002\u00020\u0001B%\u0012\u0006\u0010\u0002\u001a\u00020\u0003\u0012\n\b\u0002\u0010\u0004\u001a\u0004\u0018\u00010\u0005\u0012\n\b\u0002\u0010\u0006\u001a\u0004\u0018\u00010\u0005\u00a2\u0006\u0002\u0010\u0007J\t\u0010\r\u001a\u00020\u0003H\u00c6\u0003J\u000b\u0010\u000e\u001a\u0004\u0018\u00010\u0005H\u00c6\u0003J\u000b\u0010\u000f\u001a\u0004\u0018\u00010\u0005H\u00c6\u0003J+\u0010\u0010\u001a\u00020\u00002\b\b\u0002\u0010\u0002\u001a\u00020\u00032\n\b\u0002\u0010\u0004\u001a\u0004\u0018\u00010\u00052\n\b\u0002\u0010\u0006\u001a\u0004\u0018\u00010\u0005H\u00c6\u0001J\u0013\u0010\u0011\u001a\u00020\u00032\b\u0010\u0012\u001a\u0004\u0018\u00010\u0001H\u00d6\u0003J\t\u0010\u0013\u001a\u00020\u0014H\u00d6\u0001J\t\u0010\u0015\u001a\u00020\u0005H\u00d6\u0001R\u0011\u0010\u0002\u001a\u00020\u0003\u00a2\u0006\b\n\u0000\u001a\u0004\b\b\u0010\tR\u0013\u0010\u0006\u001a\u0004\u0018\u00010\u0005\u00a2\u0006\b\n\u0000\u001a\u0004\b\n\u0010\u000bR\u0013\u0010\u0004\u001a\u0004\u0018\u00010\u0005\u00a2\u0006\b\n\u0000\u001a\u0004\b\f\u0010\u000b\u00a8\u0006\u0016"}, d2 = {"Los/parsec/browser/ResourceBlocker$NavDecision;", "", "allow", "", "redirectUrl", "", "reason", "(ZLjava/lang/String;Ljava/lang/String;)V", "getAllow", "()Z", "getReason", "()Ljava/lang/String;", "getRedirectUrl", "component1", "component2", "component3", "copy", "equals", "other", "hashCode", "", "toString", "app_debug"})
    public static final class NavDecision {
        private final boolean allow = false;
        @org.jetbrains.annotations.Nullable()
        private final java.lang.String redirectUrl = null;
        @org.jetbrains.annotations.Nullable()
        private final java.lang.String reason = null;
        
        public NavDecision(boolean allow, @org.jetbrains.annotations.Nullable()
        java.lang.String redirectUrl, @org.jetbrains.annotations.Nullable()
        java.lang.String reason) {
            super();
        }
        
        public final boolean getAllow() {
            return false;
        }
        
        @org.jetbrains.annotations.Nullable()
        public final java.lang.String getRedirectUrl() {
            return null;
        }
        
        @org.jetbrains.annotations.Nullable()
        public final java.lang.String getReason() {
            return null;
        }
        
        public final boolean component1() {
            return false;
        }
        
        @org.jetbrains.annotations.Nullable()
        public final java.lang.String component2() {
            return null;
        }
        
        @org.jetbrains.annotations.Nullable()
        public final java.lang.String component3() {
            return null;
        }
        
        @org.jetbrains.annotations.NotNull()
        public final os.parsec.browser.ResourceBlocker.NavDecision copy(boolean allow, @org.jetbrains.annotations.Nullable()
        java.lang.String redirectUrl, @org.jetbrains.annotations.Nullable()
        java.lang.String reason) {
            return null;
        }
        
        @java.lang.Override()
        public boolean equals(@org.jetbrains.annotations.Nullable()
        java.lang.Object other) {
            return false;
        }
        
        @java.lang.Override()
        public int hashCode() {
            return 0;
        }
        
        @java.lang.Override()
        @org.jetbrains.annotations.NotNull()
        public java.lang.String toString() {
            return null;
        }
    }
    
    @kotlin.Metadata(mv = {1, 9, 0}, k = 1, xi = 48, d1 = {"\u0000\f\n\u0002\u0018\u0002\n\u0002\u0010\u0010\n\u0002\b\u0005\b\u0086\u0081\u0002\u0018\u00002\b\u0012\u0004\u0012\u00020\u00000\u0001B\u0007\b\u0002\u00a2\u0006\u0002\u0010\u0002j\u0002\b\u0003j\u0002\b\u0004j\u0002\b\u0005\u00a8\u0006\u0006"}, d2 = {"Los/parsec/browser/ResourceBlocker$SearchEngine;", "", "(Ljava/lang/String;I)V", "GOOGLE", "DUCKDUCKGO", "BING", "app_debug"})
    public static enum SearchEngine {
        /*public static final*/ GOOGLE /* = new GOOGLE() */,
        /*public static final*/ DUCKDUCKGO /* = new DUCKDUCKGO() */,
        /*public static final*/ BING /* = new BING() */;
        
        SearchEngine() {
        }
        
        @org.jetbrains.annotations.NotNull()
        public static kotlin.enums.EnumEntries<os.parsec.browser.ResourceBlocker.SearchEngine> getEntries() {
            return null;
        }
    }
}