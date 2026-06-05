package os.parsec.browser.ui;

/**
 * BrowserActivity — Main browser UI for Parsec Android.
 *
 * Architecture:
 *  - Kotlin owns all Android WebView instances (one per tab)
 *  - Rust core handles: blocking, HTTPS upgrade, sync, GPU compositor
 *  - IPC bridge: ipc() / pollEvents() connects Kotlin ↔ Rust
 *  - Tab WebViews stacked in a FrameLayout; switching = bringToFront()
 *  - Chrome UI drawn natively (no React on Android — full native Kotlin UI)
 */
@kotlin.Metadata(mv = {1, 9, 0}, k = 1, xi = 48, d1 = {"\u0000\u00c6\u0001\n\u0002\u0018\u0002\n\u0002\u0018\u0002\n\u0002\b\u0002\n\u0002\u0010\u000e\n\u0000\n\u0002\u0018\u0002\n\u0000\n\u0002\u0018\u0002\n\u0002\b\u0005\n\u0002\u0018\u0002\n\u0000\n\u0002\u0010\u000b\n\u0000\n\u0002\u0018\u0002\n\u0002\u0018\u0002\n\u0000\n\u0002\u0018\u0002\n\u0002\u0018\u0002\n\u0000\n\u0002\u0018\u0002\n\u0000\n\u0002\u0018\u0002\n\u0000\n\u0002\u0018\u0002\n\u0000\n\u0002\u0018\u0002\n\u0000\n\u0002\u0018\u0002\n\u0002\u0010\u0011\n\u0002\u0018\u0002\n\u0000\n\u0002\u0018\u0002\n\u0000\n\u0002\u0018\u0002\n\u0000\n\u0002\u0018\u0002\n\u0000\n\u0002\u0010%\n\u0002\u0018\u0002\n\u0002\b\u0002\n\u0002\u0018\u0002\n\u0000\n\u0002\u0018\u0002\n\u0002\b\u0002\n\u0002\u0010\u0002\n\u0002\b\u0005\n\u0002\u0018\u0002\n\u0002\b\r\n\u0002\u0018\u0002\n\u0002\b\u0005\n\u0002\u0010$\n\u0002\u0010\u0000\n\u0002\b\r\n\u0002\u0018\u0002\n\u0002\b\f\n\u0002\u0010\u0006\n\u0002\b!\u0018\u00002\u00020\u0001:\u0006\u0087\u0001\u0088\u0001\u0089\u0001B\u0005\u00a2\u0006\u0002\u0010\u0002J\n\u00101\u001a\u0004\u0018\u00010+H\u0002J\b\u00102\u001a\u000203H\u0002J\b\u00104\u001a\u000203H\u0002J\b\u00105\u001a\u00020\u0004H\u0002J\u0010\u00106\u001a\u00020\u00042\u0006\u00107\u001a\u00020\u0010H\u0002J\u0018\u00108\u001a\u0002092\u0006\u0010:\u001a\u00020\u00042\u0006\u0010;\u001a\u00020\u0010H\u0002J\u000e\u0010<\u001a\u0002032\u0006\u0010:\u001a\u00020\u0004J\u0010\u0010=\u001a\u0002032\u0006\u0010>\u001a\u00020\u0004H\u0002J\u0016\u0010?\u001a\u00020\u00042\u0006\u0010@\u001a\u00020\u00042\u0006\u0010;\u001a\u00020\u0010J\b\u0010A\u001a\u000203H\u0002J\u0010\u0010B\u001a\u00020\u00042\u0006\u0010@\u001a\u00020\u0004H\u0002J\b\u0010C\u001a\u00020\u0004H\u0002J\u0018\u0010D\u001a\u0002032\u0006\u0010:\u001a\u00020\u00042\u0006\u0010@\u001a\u00020\u0004H\u0002J\u0010\u0010E\u001a\u0002032\u0006\u0010F\u001a\u00020GH\u0002J\b\u0010H\u001a\u000203H\u0002J\b\u0010I\u001a\u000203H\u0002J0\u0010J\u001a\u00020G2\u0006\u0010K\u001a\u00020\u00042\u0014\b\u0002\u0010L\u001a\u000e\u0012\u0004\u0012\u00020\u0004\u0012\u0004\u0012\u00020N0M2\b\b\u0002\u0010O\u001a\u00020\u0004H\u0002J \u0010P\u001a\u0002032\u0006\u0010Q\u001a\u0002092\u0006\u0010@\u001a\u00020\u00042\u0006\u0010R\u001a\u00020\u0004H\u0002J \u0010S\u001a\u0002032\u0006\u0010Q\u001a\u0002092\u0006\u0010@\u001a\u00020\u00042\u0006\u0010T\u001a\u00020\u0004H\u0002J\u0010\u0010U\u001a\u0002032\u0006\u0010Q\u001a\u000209H\u0002J\u0010\u0010V\u001a\u0002032\u0006\u0010W\u001a\u00020\u0004H\u0002J\u0018\u0010X\u001a\u0002032\u0006\u0010:\u001a\u00020\u00042\u0006\u0010>\u001a\u00020\u0004H\u0002J\u0010\u0010Y\u001a\u00020\u00042\u0006\u0010>\u001a\u00020\u0004H\u0002J\u0012\u0010Z\u001a\u0002032\b\u0010[\u001a\u0004\u0018\u00010\\H\u0014J\b\u0010]\u001a\u000203H\u0014J\u0012\u0010^\u001a\u0002032\b\u0010_\u001a\u0004\u0018\u00010\u0016H\u0014J\b\u0010`\u001a\u000203H\u0014J\b\u0010a\u001a\u000203H\u0014J\u0010\u0010b\u001a\u0002032\u0006\u0010@\u001a\u00020\u0004H\u0002J\b\u0010c\u001a\u000203H\u0002J\u0010\u0010d\u001a\u0002032\u0006\u0010@\u001a\u00020\u0004H\u0002J\b\u0010e\u001a\u000203H\u0002J\u0018\u0010f\u001a\u0002032\u0006\u0010g\u001a\u00020\u00042\u0006\u0010h\u001a\u00020iH\u0002J\b\u0010j\u001a\u000203H\u0002J\b\u0010k\u001a\u000203H\u0002J\b\u0010l\u001a\u000203H\u0002J\b\u0010m\u001a\u000203H\u0002J\u0018\u0010n\u001a\u0002032\u0006\u0010@\u001a\u00020\u00042\u0006\u0010o\u001a\u00020\u0004H\u0002J \u0010p\u001a\u0002032\u0006\u0010q\u001a\u00020\u00042\u0006\u0010o\u001a\u00020\u00042\u0006\u0010r\u001a\u00020\u0004H\u0002J\b\u0010s\u001a\u000203H\u0002J\u0010\u0010t\u001a\u0002032\u0006\u0010:\u001a\u00020\u0004H\u0002J\b\u0010u\u001a\u000203H\u0002J\u000e\u0010v\u001a\u0002032\u0006\u0010w\u001a\u00020\u0004J\b\u0010x\u001a\u000203H\u0002J\b\u0010y\u001a\u000203H\u0002J\u0018\u0010z\u001a\u0002032\u0006\u0010@\u001a\u00020\u00042\u0006\u0010{\u001a\u00020\u0004H\u0002J\u000e\u0010|\u001a\u0002032\u0006\u0010:\u001a\u00020\u0004J\b\u0010}\u001a\u000203H\u0002J\u0010\u0010~\u001a\u0002032\u0006\u0010\u007f\u001a\u00020\u0004H\u0002J\u0011\u0010\u0080\u0001\u001a\u0002032\u0006\u0010@\u001a\u00020\u0004H\u0002J\u0014\u0010\u0081\u0001\u001a\u0002032\t\u0010\u0082\u0001\u001a\u0004\u0018\u00010+H\u0002J\t\u0010\u0083\u0001\u001a\u000203H\u0002J\u0011\u0010\u0084\u0001\u001a\u0002032\u0006\u0010o\u001a\u00020\u0004H\u0002J\u0014\u0010\u0085\u0001\u001a\u0002032\t\u0010\u0082\u0001\u001a\u0004\u0018\u00010+H\u0002J\u0011\u0010\u0086\u0001\u001a\u0002032\u0006\u0010@\u001a\u00020\u0004H\u0002R\u0010\u0010\u0003\u001a\u0004\u0018\u00010\u0004X\u0082\u000e\u00a2\u0006\u0002\n\u0000R\u000e\u0010\u0005\u001a\u00020\u0006X\u0082.\u00a2\u0006\u0002\n\u0000R\u000e\u0010\u0007\u001a\u00020\bX\u0082.\u00a2\u0006\u0002\n\u0000R\u000e\u0010\t\u001a\u00020\bX\u0082.\u00a2\u0006\u0002\n\u0000R\u000e\u0010\n\u001a\u00020\bX\u0082.\u00a2\u0006\u0002\n\u0000R\u000e\u0010\u000b\u001a\u00020\bX\u0082.\u00a2\u0006\u0002\n\u0000R\u000e\u0010\f\u001a\u00020\bX\u0082.\u00a2\u0006\u0002\n\u0000R\u000e\u0010\r\u001a\u00020\u000eX\u0082.\u00a2\u0006\u0002\n\u0000R\u000e\u0010\u000f\u001a\u00020\u0010X\u0082\u000e\u00a2\u0006\u0002\n\u0000R\u0012\u0010\u0011\u001a\u00060\u0012j\u0002`\u0013X\u0082\u0004\u00a2\u0006\u0002\n\u0000R\u0014\u0010\u0014\u001a\b\u0012\u0004\u0012\u00020\u00160\u0015X\u0082\u0004\u00a2\u0006\u0002\n\u0000R\u0010\u0010\u0017\u001a\u0004\u0018\u00010\u0018X\u0082\u000e\u00a2\u0006\u0002\n\u0000R\u000e\u0010\u0019\u001a\u00020\u001aX\u0082\u0004\u00a2\u0006\u0002\n\u0000R\u000e\u0010\u001b\u001a\u00020\u001cX\u0082\u0004\u00a2\u0006\u0002\n\u0000R\u000e\u0010\u001d\u001a\u00020\u001eX\u0082.\u00a2\u0006\u0002\n\u0000R\u001c\u0010\u001f\u001a\u0010\u0012\n\u0012\b\u0012\u0004\u0012\u00020\"0!\u0018\u00010 X\u0082\u000e\u00a2\u0006\u0002\n\u0000R\u000e\u0010#\u001a\u00020$X\u0082.\u00a2\u0006\u0002\n\u0000R\u000e\u0010%\u001a\u00020&X\u0082\u0004\u00a2\u0006\u0002\n\u0000R\u000e\u0010\'\u001a\u00020(X\u0082.\u00a2\u0006\u0002\n\u0000R\u001a\u0010)\u001a\u000e\u0012\u0004\u0012\u00020\u0004\u0012\u0004\u0012\u00020+0*X\u0082\u0004\u00a2\u0006\u0002\n\u0000R\u000e\u0010,\u001a\u00020\u0006X\u0082.\u00a2\u0006\u0002\n\u0000R\u000e\u0010-\u001a\u00020.X\u0082.\u00a2\u0006\u0002\n\u0000R\u000e\u0010/\u001a\u000200X\u0082.\u00a2\u0006\u0002\n\u0000\u00a8\u0006\u008a\u0001"}, d2 = {"Los/parsec/browser/ui/BrowserActivity;", "Landroidx/appcompat/app/AppCompatActivity;", "()V", "activeTabId", "", "bottomSheet", "Landroid/widget/LinearLayout;", "btnBack", "Landroid/widget/ImageButton;", "btnForward", "btnMenu", "btnNewTab", "btnReload", "btnTabs", "Landroid/widget/Button;", "contextMenuDirty", "", "eventPollRunnable", "Ljava/lang/Runnable;", "Lkotlinx/coroutines/Runnable;", "filePickerLauncher", "Landroidx/activity/result/ActivityResultLauncher;", "Landroid/content/Intent;", "findBarView", "Landroid/view/View;", "gson", "Lcom/google/gson/Gson;", "handler", "Landroid/os/Handler;", "lockIcon", "Landroid/widget/ImageView;", "pendingFileCallback", "Landroid/webkit/ValueCallback;", "", "Landroid/net/Uri;", "progressBar", "Landroid/widget/ProgressBar;", "scope", "Lkotlinx/coroutines/CoroutineScope;", "suggestionsList", "Landroidx/recyclerview/widget/RecyclerView;", "tabs", "", "Los/parsec/browser/ui/BrowserActivity$TabEntry;", "toolbarLayout", "urlBar", "Landroid/widget/EditText;", "webViewContainer", "Landroid/widget/FrameLayout;", "activeTab", "bindViews", "", "bookmarkCurrent", "buildNewTabHtml", "buildUserAgent", "desktopMode", "buildWebView", "Landroid/webkit/WebView;", "tabId", "incognito", "closeTab", "commitNavigate", "input", "createTab", "url", "dismissFindBar", "extractOrigin", "ghostAntiFingerprint", "handleInternalUrl", "handleRustEvent", "ev", "Lcom/google/gson/JsonObject;", "hideKeyboard", "hideSuggestions", "ipc", "cmd", "args", "", "", "id", "loadBlockedPage", "wv", "reason", "loadErrorPage", "msg", "loadNewTabPage", "loadSuggestions", "query", "navigateTab", "normalizeUrl", "onCreate", "savedInstanceState", "Landroid/os/Bundle;", "onDestroy", "onNewIntent", "intent", "onPause", "onResume", "openInExternalApp", "pollEvents", "prefetchUrl", "scheduleEventPoll", "scheduleExtensionAlarm", "name", "delayMins", "", "setupBackHandler", "setupButtons", "setupEdgeToEdge", "setupUrlBar", "shareUrl", "title", "showExtensionNotification", "notifId", "message", "showFindInPage", "showGhostBanner", "showMenuSheet", "showPanel", "panel", "showSuggestions", "showTabSwitcher", "startDownload", "filename", "switchToTab", "toggleDesktopMode", "updateExtensionBadge", "text", "updateLockIcon", "updateNavButtons", "tab", "updateTabCount", "updateTitle", "updateUrlBar", "updateUrlBarText", "ParsecWebChromeClient", "ParsecWebViewClient", "TabEntry", "app_release"})
public final class BrowserActivity extends androidx.appcompat.app.AppCompatActivity {
    @org.jetbrains.annotations.NotNull()
    private final java.util.Map<java.lang.String, os.parsec.browser.ui.BrowserActivity.TabEntry> tabs = null;
    @org.jetbrains.annotations.Nullable()
    private java.lang.String activeTabId;
    @org.jetbrains.annotations.NotNull()
    private final com.google.gson.Gson gson = null;
    @org.jetbrains.annotations.NotNull()
    private final kotlinx.coroutines.CoroutineScope scope = null;
    @org.jetbrains.annotations.NotNull()
    private final android.os.Handler handler = null;
    @org.jetbrains.annotations.NotNull()
    private final java.lang.Runnable eventPollRunnable = null;
    @org.jetbrains.annotations.Nullable()
    private android.webkit.ValueCallback<android.net.Uri[]> pendingFileCallback;
    @org.jetbrains.annotations.NotNull()
    private final androidx.activity.result.ActivityResultLauncher<android.content.Intent> filePickerLauncher = null;
    private android.widget.FrameLayout webViewContainer;
    private android.widget.LinearLayout toolbarLayout;
    private android.widget.EditText urlBar;
    private android.widget.ProgressBar progressBar;
    private android.widget.ImageButton btnBack;
    private android.widget.ImageButton btnForward;
    private android.widget.ImageButton btnReload;
    private android.widget.ImageButton btnNewTab;
    private android.widget.ImageButton btnMenu;
    private android.widget.Button btnTabs;
    private android.widget.ImageView lockIcon;
    private androidx.recyclerview.widget.RecyclerView suggestionsList;
    private android.widget.LinearLayout bottomSheet;
    
    /**
     * Tracks whether context menus need rebuilding after an extension update.
     */
    private boolean contextMenuDirty = false;
    
    /**
     * Persistent inline find-in-page bar anchored above the toolbar.
     * Shows match count, prev/next buttons, and a ✕ dismiss button.
     * Replaces the old AlertDialog approach for a Chrome-parity UX.
     */
    @org.jetbrains.annotations.Nullable()
    private android.view.View findBarView;
    
    public BrowserActivity() {
        super();
    }
    
    @java.lang.Override()
    protected void onCreate(@org.jetbrains.annotations.Nullable()
    android.os.Bundle savedInstanceState) {
    }
    
    private final void setupEdgeToEdge() {
    }
    
    @java.lang.Override()
    protected void onResume() {
    }
    
    @java.lang.Override()
    protected void onPause() {
    }
    
    @java.lang.Override()
    protected void onDestroy() {
    }
    
    @java.lang.Override()
    protected void onNewIntent(@org.jetbrains.annotations.Nullable()
    android.content.Intent intent) {
    }
    
    private final void bindViews() {
    }
    
    private final void setupUrlBar() {
    }
    
    private final void commitNavigate(java.lang.String input) {
    }
    
    private final void showSuggestions() {
    }
    
    private final void hideSuggestions() {
    }
    
    private final void loadSuggestions(java.lang.String query) {
    }
    
    private final void setupButtons() {
    }
    
    private final void setupBackHandler() {
    }
    
    @org.jetbrains.annotations.NotNull()
    public final java.lang.String createTab(@org.jetbrains.annotations.NotNull()
    java.lang.String url, boolean incognito) {
        return null;
    }
    
    public final void closeTab(@org.jetbrains.annotations.NotNull()
    java.lang.String tabId) {
    }
    
    public final void switchToTab(@org.jetbrains.annotations.NotNull()
    java.lang.String tabId) {
    }
    
    private final void navigateTab(java.lang.String tabId, java.lang.String input) {
    }
    
    private final os.parsec.browser.ui.BrowserActivity.TabEntry activeTab() {
        return null;
    }
    
    @kotlin.Suppress(names = {"SetJavaScriptEnabled"})
    private final android.webkit.WebView buildWebView(java.lang.String tabId, boolean incognito) {
        return null;
    }
    
    private final java.lang.String buildUserAgent(boolean desktopMode) {
        return null;
    }
    
    private final void handleInternalUrl(java.lang.String tabId, java.lang.String url) {
    }
    
    private final void loadNewTabPage(android.webkit.WebView wv) {
    }
    
    private final void loadBlockedPage(android.webkit.WebView wv, java.lang.String url, java.lang.String reason) {
    }
    
    private final void loadErrorPage(android.webkit.WebView wv, java.lang.String url, java.lang.String msg) {
    }
    
    private final java.lang.String buildNewTabHtml() {
        return null;
    }
    
    private final com.google.gson.JsonObject ipc(java.lang.String cmd, java.util.Map<java.lang.String, ? extends java.lang.Object> args, java.lang.String id) {
        return null;
    }
    
    private final void scheduleEventPoll() {
    }
    
    private final void pollEvents() {
    }
    
    private final void handleRustEvent(com.google.gson.JsonObject ev) {
    }
    
    private final void showExtensionNotification(java.lang.String notifId, java.lang.String title, java.lang.String message) {
    }
    
    private final void scheduleExtensionAlarm(java.lang.String name, double delayMins) {
    }
    
    private final void updateExtensionBadge(java.lang.String text) {
    }
    
    private final void updateUrlBar(os.parsec.browser.ui.BrowserActivity.TabEntry tab) {
    }
    
    private final void updateUrlBarText(java.lang.String url) {
    }
    
    private final void updateLockIcon(java.lang.String url) {
    }
    
    private final void updateNavButtons(os.parsec.browser.ui.BrowserActivity.TabEntry tab) {
    }
    
    private final void updateTitle(java.lang.String title) {
    }
    
    private final void updateTabCount() {
    }
    
    private final void showMenuSheet() {
    }
    
    private final void showTabSwitcher() {
    }
    
    public final void showPanel(@org.jetbrains.annotations.NotNull()
    java.lang.String panel) {
    }
    
    private final void showFindInPage() {
    }
    
    private final void dismissFindBar() {
    }
    
    private final void bookmarkCurrent() {
    }
    
    private final void toggleDesktopMode() {
    }
    
    private final void shareUrl(java.lang.String url, java.lang.String title) {
    }
    
    private final void openInExternalApp(java.lang.String url) {
    }
    
    private final void startDownload(java.lang.String url, java.lang.String filename) {
    }
    
    private final void prefetchUrl(java.lang.String url) {
    }
    
    private final java.lang.String normalizeUrl(java.lang.String input) {
        return null;
    }
    
    private final void hideKeyboard() {
    }
    
    private final java.lang.String extractOrigin(java.lang.String url) {
        return null;
    }
    
    /**
     * Anti-fingerprinting JavaScript injected into every incognito page.
     *
     * Neutralises:
     * - Canvas fingerprinting (adds imperceptible noise to pixel reads)
     * - WebGL fingerprinting (spoofs renderer/vendor strings)
     * - Audio fingerprinting (adds tiny noise to AudioContext output)
     * - Battery API (always returns null — real level is a tracking vector)
     * - Hardware concurrency (always returns 4 — real CPU count is fingerprint)
     * - Device memory (always returns 4 — real RAM is fingerprint)
     * - Timezone (clamped to UTC offset to prevent geo inference)
     * - Screen dimensions (reported as a common generic size)
     * - WebRTC IP exposure (overrides RTCPeerConnection to block STUN)
     * - navigator.plugins (empty — plugin list is a fingerprint)
     * - navigator.languages (single "en-US" entry)
     * - Keyboard/mouse timing APIs (clamped to prevent timing attacks)
     */
    private final java.lang.String ghostAntiFingerprint() {
        return null;
    }
    
    /**
     * Show a Ghost Mode status bar when switching to an incognito tab.
     * Displays: 🕵️ Ghost Mode • Encrypted • Keys rotate every 30min
     */
    private final void showGhostBanner(java.lang.String tabId) {
    }
    
    @kotlin.Metadata(mv = {1, 9, 0}, k = 1, xi = 48, d1 = {"\u0000X\n\u0002\u0018\u0002\n\u0002\u0018\u0002\n\u0000\n\u0002\u0010\u000e\n\u0002\b\u0002\n\u0002\u0010\u000b\n\u0000\n\u0002\u0018\u0002\n\u0002\b\u0003\n\u0002\u0018\u0002\n\u0000\n\u0002\u0010\u0002\n\u0000\n\u0002\u0018\u0002\n\u0002\b\u0002\n\u0002\u0010\b\n\u0002\b\u0002\n\u0002\u0018\u0002\n\u0002\b\u0005\n\u0002\u0018\u0002\n\u0002\u0010\u0011\n\u0002\u0018\u0002\n\u0000\n\u0002\u0018\u0002\n\u0000\b\u0082\u0004\u0018\u00002\u00020\u0001B\r\u0012\u0006\u0010\u0002\u001a\u00020\u0003\u00a2\u0006\u0002\u0010\u0004J(\u0010\u0005\u001a\u00020\u00062\u0006\u0010\u0007\u001a\u00020\b2\u0006\u0010\t\u001a\u00020\u00062\u0006\u0010\n\u001a\u00020\u00062\u0006\u0010\u000b\u001a\u00020\fH\u0016J\u0010\u0010\r\u001a\u00020\u000e2\u0006\u0010\u000f\u001a\u00020\u0010H\u0016J\u0018\u0010\u0011\u001a\u00020\u000e2\u0006\u0010\u0007\u001a\u00020\b2\u0006\u0010\u0012\u001a\u00020\u0013H\u0016J\u0018\u0010\u0014\u001a\u00020\u000e2\u0006\u0010\u0007\u001a\u00020\b2\u0006\u0010\u0015\u001a\u00020\u0016H\u0016J\u0018\u0010\u0017\u001a\u00020\u000e2\u0006\u0010\u0007\u001a\u00020\b2\u0006\u0010\u0018\u001a\u00020\u0003H\u0016J,\u0010\u0019\u001a\u00020\u00062\u0006\u0010\u001a\u001a\u00020\b2\u0012\u0010\u001b\u001a\u000e\u0012\n\u0012\b\u0012\u0004\u0012\u00020\u001e0\u001d0\u001c2\u0006\u0010\u001f\u001a\u00020 H\u0016R\u000e\u0010\u0002\u001a\u00020\u0003X\u0082\u0004\u00a2\u0006\u0002\n\u0000\u00a8\u0006!"}, d2 = {"Los/parsec/browser/ui/BrowserActivity$ParsecWebChromeClient;", "Landroid/webkit/WebChromeClient;", "tabId", "", "(Los/parsec/browser/ui/BrowserActivity;Ljava/lang/String;)V", "onCreateWindow", "", "view", "Landroid/webkit/WebView;", "isDialog", "isUserGesture", "resultMsg", "Landroid/os/Message;", "onPermissionRequest", "", "request", "Landroid/webkit/PermissionRequest;", "onProgressChanged", "newProgress", "", "onReceivedIcon", "icon", "Landroid/graphics/Bitmap;", "onReceivedTitle", "title", "onShowFileChooser", "webView", "filePathCallback", "Landroid/webkit/ValueCallback;", "", "Landroid/net/Uri;", "fileChooserParams", "Landroid/webkit/WebChromeClient$FileChooserParams;", "app_release"})
    final class ParsecWebChromeClient extends android.webkit.WebChromeClient {
        @org.jetbrains.annotations.NotNull()
        private final java.lang.String tabId = null;
        
        public ParsecWebChromeClient(@org.jetbrains.annotations.NotNull()
        java.lang.String tabId) {
            super();
        }
        
        @java.lang.Override()
        public void onProgressChanged(@org.jetbrains.annotations.NotNull()
        android.webkit.WebView view, int newProgress) {
        }
        
        @java.lang.Override()
        public void onReceivedTitle(@org.jetbrains.annotations.NotNull()
        android.webkit.WebView view, @org.jetbrains.annotations.NotNull()
        java.lang.String title) {
        }
        
        @java.lang.Override()
        public void onReceivedIcon(@org.jetbrains.annotations.NotNull()
        android.webkit.WebView view, @org.jetbrains.annotations.NotNull()
        android.graphics.Bitmap icon) {
        }
        
        @java.lang.Override()
        public boolean onCreateWindow(@org.jetbrains.annotations.NotNull()
        android.webkit.WebView view, boolean isDialog, boolean isUserGesture, @org.jetbrains.annotations.NotNull()
        android.os.Message resultMsg) {
            return false;
        }
        
        @java.lang.Override()
        public void onPermissionRequest(@org.jetbrains.annotations.NotNull()
        android.webkit.PermissionRequest request) {
        }
        
        @java.lang.Override()
        public boolean onShowFileChooser(@org.jetbrains.annotations.NotNull()
        android.webkit.WebView webView, @org.jetbrains.annotations.NotNull()
        android.webkit.ValueCallback<android.net.Uri[]> filePathCallback, @org.jetbrains.annotations.NotNull()
        android.webkit.WebChromeClient.FileChooserParams fileChooserParams) {
            return false;
        }
    }
    
    @kotlin.Metadata(mv = {1, 9, 0}, k = 1, xi = 48, d1 = {"\u0000H\n\u0002\u0018\u0002\n\u0002\u0018\u0002\n\u0000\n\u0002\u0010\u000e\n\u0000\n\u0002\u0010\u000b\n\u0002\b\u0002\n\u0002\u0010\u0002\n\u0000\n\u0002\u0018\u0002\n\u0002\b\u0003\n\u0002\u0018\u0002\n\u0002\b\u0002\n\u0002\u0018\u0002\n\u0000\n\u0002\u0018\u0002\n\u0000\n\u0002\u0018\u0002\n\u0000\n\u0002\u0018\u0002\n\u0002\b\u0002\b\u0082\u0004\u0018\u00002\u00020\u0001B\u0015\u0012\u0006\u0010\u0002\u001a\u00020\u0003\u0012\u0006\u0010\u0004\u001a\u00020\u0005\u00a2\u0006\u0002\u0010\u0006J\u0018\u0010\u0007\u001a\u00020\b2\u0006\u0010\t\u001a\u00020\n2\u0006\u0010\u000b\u001a\u00020\u0003H\u0016J\"\u0010\f\u001a\u00020\b2\u0006\u0010\t\u001a\u00020\n2\u0006\u0010\u000b\u001a\u00020\u00032\b\u0010\r\u001a\u0004\u0018\u00010\u000eH\u0016J \u0010\u000f\u001a\u00020\b2\u0006\u0010\t\u001a\u00020\n2\u0006\u0010\u0010\u001a\u00020\u00112\u0006\u0010\u0012\u001a\u00020\u0013H\u0016J\u001a\u0010\u0014\u001a\u0004\u0018\u00010\u00152\u0006\u0010\t\u001a\u00020\n2\u0006\u0010\u0016\u001a\u00020\u0017H\u0016J\u0018\u0010\u0018\u001a\u00020\u00052\u0006\u0010\t\u001a\u00020\n2\u0006\u0010\u0016\u001a\u00020\u0017H\u0016R\u000e\u0010\u0004\u001a\u00020\u0005X\u0082\u0004\u00a2\u0006\u0002\n\u0000R\u000e\u0010\u0002\u001a\u00020\u0003X\u0082\u0004\u00a2\u0006\u0002\n\u0000\u00a8\u0006\u0019"}, d2 = {"Los/parsec/browser/ui/BrowserActivity$ParsecWebViewClient;", "Landroid/webkit/WebViewClient;", "tabId", "", "incognito", "", "(Los/parsec/browser/ui/BrowserActivity;Ljava/lang/String;Z)V", "onPageFinished", "", "view", "Landroid/webkit/WebView;", "url", "onPageStarted", "favicon", "Landroid/graphics/Bitmap;", "onReceivedSslError", "handler", "Landroid/webkit/SslErrorHandler;", "error", "Landroid/net/http/SslError;", "shouldInterceptRequest", "Landroid/webkit/WebResourceResponse;", "request", "Landroid/webkit/WebResourceRequest;", "shouldOverrideUrlLoading", "app_release"})
    final class ParsecWebViewClient extends android.webkit.WebViewClient {
        @org.jetbrains.annotations.NotNull()
        private final java.lang.String tabId = null;
        private final boolean incognito = false;
        
        public ParsecWebViewClient(@org.jetbrains.annotations.NotNull()
        java.lang.String tabId, boolean incognito) {
            super();
        }
        
        @java.lang.Override()
        public boolean shouldOverrideUrlLoading(@org.jetbrains.annotations.NotNull()
        android.webkit.WebView view, @org.jetbrains.annotations.NotNull()
        android.webkit.WebResourceRequest request) {
            return false;
        }
        
        @java.lang.Override()
        @org.jetbrains.annotations.Nullable()
        public android.webkit.WebResourceResponse shouldInterceptRequest(@org.jetbrains.annotations.NotNull()
        android.webkit.WebView view, @org.jetbrains.annotations.NotNull()
        android.webkit.WebResourceRequest request) {
            return null;
        }
        
        @java.lang.Override()
        public void onPageStarted(@org.jetbrains.annotations.NotNull()
        android.webkit.WebView view, @org.jetbrains.annotations.NotNull()
        java.lang.String url, @org.jetbrains.annotations.Nullable()
        android.graphics.Bitmap favicon) {
        }
        
        @java.lang.Override()
        public void onPageFinished(@org.jetbrains.annotations.NotNull()
        android.webkit.WebView view, @org.jetbrains.annotations.NotNull()
        java.lang.String url) {
        }
        
        @java.lang.Override()
        public void onReceivedSslError(@org.jetbrains.annotations.NotNull()
        android.webkit.WebView view, @org.jetbrains.annotations.NotNull()
        android.webkit.SslErrorHandler handler, @org.jetbrains.annotations.NotNull()
        android.net.http.SslError error) {
        }
    }
    
    @kotlin.Metadata(mv = {1, 9, 0}, k = 1, xi = 48, d1 = {"\u0000(\n\u0002\u0018\u0002\n\u0002\u0010\u0000\n\u0000\n\u0002\u0010\u000e\n\u0000\n\u0002\u0018\u0002\n\u0002\b\u0004\n\u0002\u0010\u000b\n\u0002\b\"\n\u0002\u0010\b\n\u0002\b\u0002\b\u0086\b\u0018\u00002\u00020\u0001BQ\u0012\u0006\u0010\u0002\u001a\u00020\u0003\u0012\u0006\u0010\u0004\u001a\u00020\u0005\u0012\b\b\u0002\u0010\u0006\u001a\u00020\u0003\u0012\b\b\u0002\u0010\u0007\u001a\u00020\u0003\u0012\b\b\u0002\u0010\b\u001a\u00020\u0003\u0012\b\b\u0002\u0010\t\u001a\u00020\n\u0012\b\b\u0002\u0010\u000b\u001a\u00020\n\u0012\b\b\u0002\u0010\f\u001a\u00020\n\u00a2\u0006\u0002\u0010\rJ\t\u0010!\u001a\u00020\u0003H\u00c6\u0003J\t\u0010\"\u001a\u00020\u0005H\u00c6\u0003J\t\u0010#\u001a\u00020\u0003H\u00c6\u0003J\t\u0010$\u001a\u00020\u0003H\u00c6\u0003J\t\u0010%\u001a\u00020\u0003H\u00c6\u0003J\t\u0010&\u001a\u00020\nH\u00c6\u0003J\t\u0010\'\u001a\u00020\nH\u00c6\u0003J\t\u0010(\u001a\u00020\nH\u00c6\u0003JY\u0010)\u001a\u00020\u00002\b\b\u0002\u0010\u0002\u001a\u00020\u00032\b\b\u0002\u0010\u0004\u001a\u00020\u00052\b\b\u0002\u0010\u0006\u001a\u00020\u00032\b\b\u0002\u0010\u0007\u001a\u00020\u00032\b\b\u0002\u0010\b\u001a\u00020\u00032\b\b\u0002\u0010\t\u001a\u00020\n2\b\b\u0002\u0010\u000b\u001a\u00020\n2\b\b\u0002\u0010\f\u001a\u00020\nH\u00c6\u0001J\u0013\u0010*\u001a\u00020\n2\b\u0010+\u001a\u0004\u0018\u00010\u0001H\u00d6\u0003J\t\u0010,\u001a\u00020-H\u00d6\u0001J\t\u0010.\u001a\u00020\u0003H\u00d6\u0001R\u001a\u0010\b\u001a\u00020\u0003X\u0086\u000e\u00a2\u0006\u000e\n\u0000\u001a\u0004\b\u000e\u0010\u000f\"\u0004\b\u0010\u0010\u0011R\u0011\u0010\u0002\u001a\u00020\u0003\u00a2\u0006\b\n\u0000\u001a\u0004\b\u0012\u0010\u000fR\u001a\u0010\u000b\u001a\u00020\nX\u0086\u000e\u00a2\u0006\u000e\n\u0000\u001a\u0004\b\u0013\u0010\u0014\"\u0004\b\u0015\u0010\u0016R\u001a\u0010\t\u001a\u00020\nX\u0086\u000e\u00a2\u0006\u000e\n\u0000\u001a\u0004\b\u0017\u0010\u0014\"\u0004\b\u0018\u0010\u0016R\u001a\u0010\f\u001a\u00020\nX\u0086\u000e\u00a2\u0006\u000e\n\u0000\u001a\u0004\b\u0019\u0010\u0014\"\u0004\b\u001a\u0010\u0016R\u001a\u0010\u0007\u001a\u00020\u0003X\u0086\u000e\u00a2\u0006\u000e\n\u0000\u001a\u0004\b\u001b\u0010\u000f\"\u0004\b\u001c\u0010\u0011R\u001a\u0010\u0006\u001a\u00020\u0003X\u0086\u000e\u00a2\u0006\u000e\n\u0000\u001a\u0004\b\u001d\u0010\u000f\"\u0004\b\u001e\u0010\u0011R\u0011\u0010\u0004\u001a\u00020\u0005\u00a2\u0006\b\n\u0000\u001a\u0004\b\u001f\u0010 \u00a8\u0006/"}, d2 = {"Los/parsec/browser/ui/BrowserActivity$TabEntry;", "", "id", "", "webView", "Landroid/webkit/WebView;", "url", "title", "favicon", "loading", "", "incognito", "pinned", "(Ljava/lang/String;Landroid/webkit/WebView;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;ZZZ)V", "getFavicon", "()Ljava/lang/String;", "setFavicon", "(Ljava/lang/String;)V", "getId", "getIncognito", "()Z", "setIncognito", "(Z)V", "getLoading", "setLoading", "getPinned", "setPinned", "getTitle", "setTitle", "getUrl", "setUrl", "getWebView", "()Landroid/webkit/WebView;", "component1", "component2", "component3", "component4", "component5", "component6", "component7", "component8", "copy", "equals", "other", "hashCode", "", "toString", "app_release"})
    public static final class TabEntry {
        @org.jetbrains.annotations.NotNull()
        private final java.lang.String id = null;
        @org.jetbrains.annotations.NotNull()
        private final android.webkit.WebView webView = null;
        @org.jetbrains.annotations.NotNull()
        private java.lang.String url;
        @org.jetbrains.annotations.NotNull()
        private java.lang.String title;
        @org.jetbrains.annotations.NotNull()
        private java.lang.String favicon;
        private boolean loading;
        private boolean incognito;
        private boolean pinned;
        
        public TabEntry(@org.jetbrains.annotations.NotNull()
        java.lang.String id, @org.jetbrains.annotations.NotNull()
        android.webkit.WebView webView, @org.jetbrains.annotations.NotNull()
        java.lang.String url, @org.jetbrains.annotations.NotNull()
        java.lang.String title, @org.jetbrains.annotations.NotNull()
        java.lang.String favicon, boolean loading, boolean incognito, boolean pinned) {
            super();
        }
        
        @org.jetbrains.annotations.NotNull()
        public final java.lang.String getId() {
            return null;
        }
        
        @org.jetbrains.annotations.NotNull()
        public final android.webkit.WebView getWebView() {
            return null;
        }
        
        @org.jetbrains.annotations.NotNull()
        public final java.lang.String getUrl() {
            return null;
        }
        
        public final void setUrl(@org.jetbrains.annotations.NotNull()
        java.lang.String p0) {
        }
        
        @org.jetbrains.annotations.NotNull()
        public final java.lang.String getTitle() {
            return null;
        }
        
        public final void setTitle(@org.jetbrains.annotations.NotNull()
        java.lang.String p0) {
        }
        
        @org.jetbrains.annotations.NotNull()
        public final java.lang.String getFavicon() {
            return null;
        }
        
        public final void setFavicon(@org.jetbrains.annotations.NotNull()
        java.lang.String p0) {
        }
        
        public final boolean getLoading() {
            return false;
        }
        
        public final void setLoading(boolean p0) {
        }
        
        public final boolean getIncognito() {
            return false;
        }
        
        public final void setIncognito(boolean p0) {
        }
        
        public final boolean getPinned() {
            return false;
        }
        
        public final void setPinned(boolean p0) {
        }
        
        @org.jetbrains.annotations.NotNull()
        public final java.lang.String component1() {
            return null;
        }
        
        @org.jetbrains.annotations.NotNull()
        public final android.webkit.WebView component2() {
            return null;
        }
        
        @org.jetbrains.annotations.NotNull()
        public final java.lang.String component3() {
            return null;
        }
        
        @org.jetbrains.annotations.NotNull()
        public final java.lang.String component4() {
            return null;
        }
        
        @org.jetbrains.annotations.NotNull()
        public final java.lang.String component5() {
            return null;
        }
        
        public final boolean component6() {
            return false;
        }
        
        public final boolean component7() {
            return false;
        }
        
        public final boolean component8() {
            return false;
        }
        
        @org.jetbrains.annotations.NotNull()
        public final os.parsec.browser.ui.BrowserActivity.TabEntry copy(@org.jetbrains.annotations.NotNull()
        java.lang.String id, @org.jetbrains.annotations.NotNull()
        android.webkit.WebView webView, @org.jetbrains.annotations.NotNull()
        java.lang.String url, @org.jetbrains.annotations.NotNull()
        java.lang.String title, @org.jetbrains.annotations.NotNull()
        java.lang.String favicon, boolean loading, boolean incognito, boolean pinned) {
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
}