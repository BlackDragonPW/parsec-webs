package os.parsec.browser.service;

/**
 * DownloadService — Foreground service for file downloads.
 *
 * Launched by BrowserActivity.startDownload() via Intent extras:
 *  "url"         → download URL
 *  "filename"    → suggested filename
 *
 * Supports pause, resume (HTTP Range), and cancel.
 * Send an Intent with action "PAUSE"/"RESUME"/"CANCEL" and
 * extra "download_id" to control an active download.
 * RESUME also requires "url" and "dest" extras.
 */
@kotlin.Metadata(mv = {1, 9, 0}, k = 1, xi = 48, d1 = {"\u0000p\n\u0002\u0018\u0002\n\u0002\u0018\u0002\n\u0002\b\u0002\n\u0002\u0010\u000e\n\u0000\n\u0002\u0010\b\n\u0000\n\u0002\u0018\u0002\n\u0000\n\u0002\u0010#\n\u0000\n\u0002\u0010%\n\u0002\u0018\u0002\n\u0000\n\u0002\u0018\u0002\n\u0000\n\u0002\u0018\u0002\n\u0002\b\u0006\n\u0002\u0018\u0002\n\u0000\n\u0002\u0018\u0002\n\u0002\b\u0003\n\u0002\u0010\u0002\n\u0002\b\u0004\n\u0002\u0018\u0002\n\u0002\b\b\n\u0002\u0018\u0002\n\u0000\n\u0002\u0018\u0002\n\u0002\b\n\n\u0002\u0010\t\n\u0002\b\t\u0018\u00002\u00020\u0001:\u0001AB\u0005\u00a2\u0006\u0002\u0010\u0002J\u0018\u0010\u0019\u001a\u00020\u001a2\u0006\u0010\u001b\u001a\u00020\u00042\u0006\u0010\u001c\u001a\u00020\u0006H\u0002J\u0010\u0010\u001d\u001a\u00020\u001e2\u0006\u0010\u001f\u001a\u00020\u0004H\u0002J \u0010 \u001a\u00020\u001e2\u0006\u0010!\u001a\u00020\u00062\u0006\u0010\u001b\u001a\u00020\u00042\u0006\u0010\"\u001a\u00020#H\u0002J\b\u0010$\u001a\u00020\u001eH\u0002J&\u0010%\u001a\u00020\u001e2\u0006\u0010&\u001a\u00020\u00042\u0006\u0010\u001b\u001a\u00020\u00042\u0006\u0010!\u001a\u00020\u0006H\u0082@\u00a2\u0006\u0002\u0010\'J \u0010(\u001a\u00020\u001e2\u0006\u0010!\u001a\u00020\u00062\u0006\u0010\u001b\u001a\u00020\u00042\u0006\u0010)\u001a\u00020\u0004H\u0002J\u0010\u0010*\u001a\u00020\u00042\u0006\u0010&\u001a\u00020\u0004H\u0002J\u0012\u0010+\u001a\u00020,2\b\u0010-\u001a\u0004\u0018\u00010.H\u0016J\b\u0010/\u001a\u00020\u001eH\u0016J\b\u00100\u001a\u00020\u001eH\u0016J\"\u00101\u001a\u00020\u00062\b\u0010-\u001a\u0004\u0018\u00010.2\u0006\u00102\u001a\u00020\u00062\u0006\u00103\u001a\u00020\u0006H\u0016J\u0010\u00104\u001a\u00020\u001e2\u0006\u0010\u001f\u001a\u00020\u0004H\u0002J \u00105\u001a\u00020\u001e2\u0006\u0010\u001f\u001a\u00020\u00042\u0006\u0010&\u001a\u00020\u00042\u0006\u00106\u001a\u00020\u0004H\u0002J*\u00107\u001a\u00020\u001e2\u0006\u0010\u001f\u001a\u00020\u00042\u0006\u0010&\u001a\u00020\u00042\u0006\u00106\u001a\u00020\u00042\b\b\u0002\u00108\u001a\u000209H\u0002J \u0010:\u001a\u00020\u001e2\u0006\u0010!\u001a\u00020\u00062\u0006\u0010;\u001a\u00020\u00042\u0006\u0010<\u001a\u00020\u0004H\u0002J\u0018\u0010=\u001a\u00020#2\u0006\u0010>\u001a\u00020#2\u0006\u0010?\u001a\u00020\u0004H\u0002J \u0010@\u001a\u00020\u001e2\u0006\u0010!\u001a\u00020\u00062\u0006\u0010\u001b\u001a\u00020\u00042\u0006\u0010\u001c\u001a\u00020\u0006H\u0002R\u000e\u0010\u0003\u001a\u00020\u0004X\u0082D\u00a2\u0006\u0002\n\u0000R\u000e\u0010\u0005\u001a\u00020\u0006X\u0082D\u00a2\u0006\u0002\n\u0000R\u0012\u0010\u0007\u001a\u00060\bR\u00020\u0000X\u0082\u0004\u00a2\u0006\u0002\n\u0000R\u0014\u0010\t\u001a\b\u0012\u0004\u0012\u00020\u00040\nX\u0082\u0004\u00a2\u0006\u0002\n\u0000R\u001a\u0010\u000b\u001a\u000e\u0012\u0004\u0012\u00020\u0004\u0012\u0004\u0012\u00020\r0\fX\u0082\u0004\u00a2\u0006\u0002\n\u0000R\u000e\u0010\u000e\u001a\u00020\u000fX\u0082\u0004\u00a2\u0006\u0002\n\u0000R\u001b\u0010\u0010\u001a\u00020\u00118BX\u0082\u0084\u0002\u00a2\u0006\f\n\u0004\b\u0014\u0010\u0015\u001a\u0004\b\u0012\u0010\u0013R\u0014\u0010\u0016\u001a\b\u0012\u0004\u0012\u00020\u00040\nX\u0082\u0004\u00a2\u0006\u0002\n\u0000R\u000e\u0010\u0017\u001a\u00020\u0018X\u0082\u0004\u00a2\u0006\u0002\n\u0000\u00a8\u0006B"}, d2 = {"Los/parsec/browser/service/DownloadService;", "Landroid/app/Service;", "()V", "CHANNEL_ID", "", "NOTIF_BASE", "", "binder", "Los/parsec/browser/service/DownloadService$LocalBinder;", "cancelledDownloads", "", "downloadJobs", "", "Lkotlinx/coroutines/Job;", "http", "Lokhttp3/OkHttpClient;", "nm", "Landroid/app/NotificationManager;", "getNm", "()Landroid/app/NotificationManager;", "nm$delegate", "Lkotlin/Lazy;", "pausedDownloads", "scope", "Lkotlinx/coroutines/CoroutineScope;", "buildProgressNotif", "Landroid/app/Notification;", "filename", "progress", "cancelDownload", "", "downloadId", "completeNotif", "notifId", "file", "Ljava/io/File;", "createNotificationChannel", "download", "url", "(Ljava/lang/String;Ljava/lang/String;ILkotlin/coroutines/Continuation;)Ljava/lang/Object;", "errorNotif", "error", "guessFilename", "onBind", "Landroid/os/IBinder;", "intent", "Landroid/content/Intent;", "onCreate", "onDestroy", "onStartCommand", "flags", "startId", "pauseDownload", "resumeDownload", "dest", "startResumableJob", "resumeFrom", "", "statusNotif", "title", "text", "uniqueFile", "dir", "name", "updateProgressNotif", "LocalBinder", "app_release"})
public final class DownloadService extends android.app.Service {
    @org.jetbrains.annotations.NotNull()
    private final kotlinx.coroutines.CoroutineScope scope = null;
    @org.jetbrains.annotations.NotNull()
    private final okhttp3.OkHttpClient http = null;
    @org.jetbrains.annotations.NotNull()
    private final os.parsec.browser.service.DownloadService.LocalBinder binder = null;
    @org.jetbrains.annotations.NotNull()
    private final java.lang.String CHANNEL_ID = "parsec_downloads";
    private final int NOTIF_BASE = 1000;
    @org.jetbrains.annotations.NotNull()
    private final java.util.Map<java.lang.String, kotlinx.coroutines.Job> downloadJobs = null;
    @org.jetbrains.annotations.NotNull()
    private final java.util.Set<java.lang.String> pausedDownloads = null;
    @org.jetbrains.annotations.NotNull()
    private final java.util.Set<java.lang.String> cancelledDownloads = null;
    @org.jetbrains.annotations.NotNull()
    private final kotlin.Lazy nm$delegate = null;
    
    public DownloadService() {
        super();
    }
    
    private final android.app.NotificationManager getNm() {
        return null;
    }
    
    @java.lang.Override()
    @org.jetbrains.annotations.NotNull()
    public android.os.IBinder onBind(@org.jetbrains.annotations.Nullable()
    android.content.Intent intent) {
        return null;
    }
    
    @java.lang.Override()
    public void onCreate() {
    }
    
    @java.lang.Override()
    public int onStartCommand(@org.jetbrains.annotations.Nullable()
    android.content.Intent intent, int flags, int startId) {
        return 0;
    }
    
    private final java.lang.Object download(java.lang.String url, java.lang.String filename, int notifId, kotlin.coroutines.Continuation<? super kotlin.Unit> $completion) {
        return null;
    }
    
    private final void pauseDownload(java.lang.String downloadId) {
    }
    
    private final void resumeDownload(java.lang.String downloadId, java.lang.String url, java.lang.String dest) {
    }
    
    private final void cancelDownload(java.lang.String downloadId) {
    }
    
    private final void startResumableJob(java.lang.String downloadId, java.lang.String url, java.lang.String dest, long resumeFrom) {
    }
    
    private final void createNotificationChannel() {
    }
    
    private final android.app.Notification buildProgressNotif(java.lang.String filename, int progress) {
        return null;
    }
    
    private final void updateProgressNotif(int notifId, java.lang.String filename, int progress) {
    }
    
    private final void statusNotif(int notifId, java.lang.String title, java.lang.String text) {
    }
    
    private final void completeNotif(int notifId, java.lang.String filename, java.io.File file) {
    }
    
    private final void errorNotif(int notifId, java.lang.String filename, java.lang.String error) {
    }
    
    private final java.lang.String guessFilename(java.lang.String url) {
        return null;
    }
    
    private final java.io.File uniqueFile(java.io.File dir, java.lang.String name) {
        return null;
    }
    
    @java.lang.Override()
    public void onDestroy() {
    }
    
    @kotlin.Metadata(mv = {1, 9, 0}, k = 1, xi = 48, d1 = {"\u0000\u0012\n\u0002\u0018\u0002\n\u0002\u0018\u0002\n\u0002\b\u0002\n\u0002\u0018\u0002\n\u0000\b\u0086\u0004\u0018\u00002\u00020\u0001B\u0005\u00a2\u0006\u0002\u0010\u0002J\u0006\u0010\u0003\u001a\u00020\u0004\u00a8\u0006\u0005"}, d2 = {"Los/parsec/browser/service/DownloadService$LocalBinder;", "Landroid/os/Binder;", "(Los/parsec/browser/service/DownloadService;)V", "getService", "Los/parsec/browser/service/DownloadService;", "app_release"})
    public final class LocalBinder extends android.os.Binder {
        
        public LocalBinder() {
            super();
        }
        
        @org.jetbrains.annotations.NotNull()
        public final os.parsec.browser.service.DownloadService getService() {
            return null;
        }
    }
}