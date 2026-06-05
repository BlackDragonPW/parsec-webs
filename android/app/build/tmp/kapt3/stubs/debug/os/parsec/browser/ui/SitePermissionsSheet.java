package os.parsec.browser.ui;

/**
 * SitePermissionsSheet — per-origin permission management.
 *
 * Shows all permission states (Camera, Mic, Location, Notifications,
 * Autoplay, Popups, Clipboard, Fullscreen) for the current site.
 * Each permission has Allow / Ask / Block radio buttons.
 *
 * Equivalent to Chrome's "Site Information" → "Site settings" panel.
 * Unlike Chrome, Parsec shows autoplay and clipboard controls too,
 * and defaults autoplay and clipboard to Block.
 */
@kotlin.Metadata(mv = {1, 9, 0}, k = 1, xi = 48, d1 = {"\u0000J\n\u0002\u0018\u0002\n\u0002\u0018\u0002\n\u0002\b\u0002\n\u0002\u0010\u000e\n\u0002\b\u0005\n\u0002\u0010\u0002\n\u0002\b\u0003\n\u0002\u0018\u0002\n\u0000\n\u0002\u0018\u0002\n\u0002\b\u0003\n\u0002\u0010\u000b\n\u0000\n\u0002\u0018\u0002\n\u0000\n\u0002\u0018\u0002\n\u0000\n\u0002\u0018\u0002\n\u0000\n\u0002\u0018\u0002\n\u0002\b\u0002\u0018\u0000 \u001d2\u00020\u0001:\u0001\u001dB\u0005\u00a2\u0006\u0002\u0010\u0002J\u0018\u0010\t\u001a\u00020\n2\u0006\u0010\u000b\u001a\u00020\u00042\u0006\u0010\f\u001a\u00020\u0004H\u0002J0\u0010\r\u001a\u00020\u000e2\u0006\u0010\u000f\u001a\u00020\u00102\u0006\u0010\u0011\u001a\u00020\u00042\u0006\u0010\u0012\u001a\u00020\u00042\u0006\u0010\u000b\u001a\u00020\u00042\u0006\u0010\u0013\u001a\u00020\u0014H\u0002J$\u0010\u0015\u001a\u00020\u00162\u0006\u0010\u0017\u001a\u00020\u00182\b\u0010\u0019\u001a\u0004\u0018\u00010\u001a2\b\u0010\u001b\u001a\u0004\u0018\u00010\u001cH\u0016R\u0014\u0010\u0003\u001a\u00020\u00048BX\u0082\u0004\u00a2\u0006\u0006\u001a\u0004\b\u0005\u0010\u0006R\u0014\u0010\u0007\u001a\u00020\u00048BX\u0082\u0004\u00a2\u0006\u0006\u001a\u0004\b\b\u0010\u0006\u00a8\u0006\u001e"}, d2 = {"Los/parsec/browser/ui/SitePermissionsSheet;", "Lcom/google/android/material/bottomsheet/BottomSheetDialogFragment;", "()V", "origin", "", "getOrigin", "()Ljava/lang/String;", "pageTitle", "getPageTitle", "applyPermission", "", "key", "state", "buildPermRow", "Landroid/widget/LinearLayout;", "ctx", "Landroid/content/Context;", "label", "emoji", "defaultBlock", "", "onCreateView", "Landroid/view/View;", "inflater", "Landroid/view/LayoutInflater;", "container", "Landroid/view/ViewGroup;", "savedInstanceState", "Landroid/os/Bundle;", "Companion", "app_debug"})
public final class SitePermissionsSheet extends com.google.android.material.bottomsheet.BottomSheetDialogFragment {
    @org.jetbrains.annotations.NotNull()
    private static final java.lang.String ARG_ORIGIN = "origin";
    @org.jetbrains.annotations.NotNull()
    private static final java.lang.String ARG_TITLE = "title";
    @org.jetbrains.annotations.NotNull()
    public static final os.parsec.browser.ui.SitePermissionsSheet.Companion Companion = null;
    
    public SitePermissionsSheet() {
        super();
    }
    
    private final java.lang.String getOrigin() {
        return null;
    }
    
    private final java.lang.String getPageTitle() {
        return null;
    }
    
    @java.lang.Override()
    @org.jetbrains.annotations.NotNull()
    public android.view.View onCreateView(@org.jetbrains.annotations.NotNull()
    android.view.LayoutInflater inflater, @org.jetbrains.annotations.Nullable()
    android.view.ViewGroup container, @org.jetbrains.annotations.Nullable()
    android.os.Bundle savedInstanceState) {
        return null;
    }
    
    private final android.widget.LinearLayout buildPermRow(android.content.Context ctx, java.lang.String label, java.lang.String emoji, java.lang.String key, boolean defaultBlock) {
        return null;
    }
    
    private final void applyPermission(java.lang.String key, java.lang.String state) {
    }
    
    @kotlin.Metadata(mv = {1, 9, 0}, k = 1, xi = 48, d1 = {"\u0000\u001c\n\u0002\u0018\u0002\n\u0002\u0010\u0000\n\u0002\b\u0002\n\u0002\u0010\u000e\n\u0002\b\u0002\n\u0002\u0018\u0002\n\u0002\b\u0003\b\u0086\u0003\u0018\u00002\u00020\u0001B\u0007\b\u0002\u00a2\u0006\u0002\u0010\u0002J\u0016\u0010\u0006\u001a\u00020\u00072\u0006\u0010\b\u001a\u00020\u00042\u0006\u0010\t\u001a\u00020\u0004R\u000e\u0010\u0003\u001a\u00020\u0004X\u0082T\u00a2\u0006\u0002\n\u0000R\u000e\u0010\u0005\u001a\u00020\u0004X\u0082T\u00a2\u0006\u0002\n\u0000\u00a8\u0006\n"}, d2 = {"Los/parsec/browser/ui/SitePermissionsSheet$Companion;", "", "()V", "ARG_ORIGIN", "", "ARG_TITLE", "newInstance", "Los/parsec/browser/ui/SitePermissionsSheet;", "origin", "pageTitle", "app_debug"})
    public static final class Companion {
        
        private Companion() {
            super();
        }
        
        @org.jetbrains.annotations.NotNull()
        public final os.parsec.browser.ui.SitePermissionsSheet newInstance(@org.jetbrains.annotations.NotNull()
        java.lang.String origin, @org.jetbrains.annotations.NotNull()
        java.lang.String pageTitle) {
            return null;
        }
    }
}