package os.parsec.browser.ui;

/**
 * JS bridge for the new-tab page (parsec://newtab).
 * Lets the in-page search box call back into Kotlin without leaving the WebView sandbox.
 */
@kotlin.Metadata(mv = {1, 9, 0}, k = 1, xi = 48, d1 = {"\u0000 \n\u0002\u0018\u0002\n\u0002\u0010\u0000\n\u0000\n\u0002\u0018\u0002\n\u0000\n\u0002\u0010\u000e\n\u0002\b\u0003\n\u0002\u0010\u0002\n\u0002\b\u0004\u0018\u00002\u00020\u0001B\u0015\u0012\u0006\u0010\u0002\u001a\u00020\u0003\u0012\u0006\u0010\u0004\u001a\u00020\u0005\u00a2\u0006\u0002\u0010\u0006J\b\u0010\u0007\u001a\u00020\u0005H\u0007J\u0010\u0010\b\u001a\u00020\t2\u0006\u0010\n\u001a\u00020\u0005H\u0007J\u0010\u0010\u000b\u001a\u00020\t2\u0006\u0010\f\u001a\u00020\u0005H\u0007R\u000e\u0010\u0002\u001a\u00020\u0003X\u0082\u0004\u00a2\u0006\u0002\n\u0000R\u000e\u0010\u0004\u001a\u00020\u0005X\u0082\u0004\u00a2\u0006\u0002\n\u0000\u00a8\u0006\r"}, d2 = {"Los/parsec/browser/ui/ParsecJsBridge;", "", "activity", "Los/parsec/browser/ui/BrowserActivity;", "tabId", "", "(Los/parsec/browser/ui/BrowserActivity;Ljava/lang/String;)V", "getPrivacyStats", "openUrl", "", "url", "search", "query", "app_debug"})
public final class ParsecJsBridge {
    @org.jetbrains.annotations.NotNull()
    private final os.parsec.browser.ui.BrowserActivity activity = null;
    @org.jetbrains.annotations.NotNull()
    private final java.lang.String tabId = null;
    
    public ParsecJsBridge(@org.jetbrains.annotations.NotNull()
    os.parsec.browser.ui.BrowserActivity activity, @org.jetbrains.annotations.NotNull()
    java.lang.String tabId) {
        super();
    }
    
    @android.webkit.JavascriptInterface()
    public final void search(@org.jetbrains.annotations.NotNull()
    java.lang.String query) {
    }
    
    @android.webkit.JavascriptInterface()
    public final void openUrl(@org.jetbrains.annotations.NotNull()
    java.lang.String url) {
    }
    
    @android.webkit.JavascriptInterface()
    @org.jetbrains.annotations.NotNull()
    public final java.lang.String getPrivacyStats() {
        return null;
    }
}