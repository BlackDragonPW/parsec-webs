package os.parsec.browser.service

import android.app.*
import android.content.Intent
import android.os.*
import android.webkit.MimeTypeMap
import androidx.core.app.NotificationCompat
import kotlinx.coroutines.*
import okhttp3.*
import java.io.*
import android.os.Environment

/**
 * DownloadService — Foreground service for file downloads.
 *
 * Launched by BrowserActivity.startDownload() via Intent extras:
 *   "url"         → download URL
 *   "filename"    → suggested filename
 *
 * Supports pause, resume (HTTP Range), and cancel.
 * Send an Intent with action "PAUSE"/"RESUME"/"CANCEL" and
 * extra "download_id" to control an active download.
 * RESUME also requires "url" and "dest" extras.
 */
class DownloadService : Service() {

    // ── Fields ────────────────────────────────────────────────────────────────

    private val scope  = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val http   = OkHttpClient.Builder().build()
    private val binder = LocalBinder()

    private val CHANNEL_ID = "parsec_downloads"
    private val NOTIF_BASE = 1000

    // Job registry for pause/resume/cancel
    private val downloadJobs       = mutableMapOf<String, Job>()
    private val pausedDownloads    = mutableSetOf<String>()
    private val cancelledDownloads = mutableSetOf<String>()

    // Lazy notif manager — avoids getSystemService in field initialiser
    private val nm: NotificationManager by lazy {
        getSystemService(NOTIFICATION_SERVICE) as NotificationManager
    }

    inner class LocalBinder : Binder() {
        fun getService(): DownloadService = this@DownloadService
    }

    override fun onBind(intent: Intent?): IBinder = binder

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
    }

    // ── onStartCommand ────────────────────────────────────────────────────────

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        // Handle control actions (pause / resume / cancel)
        when (intent?.action) {
            "PAUSE" -> {
                intent.getStringExtra("download_id")?.let { pauseDownload(it) }
                return START_STICKY
            }
            "RESUME" -> {
                val id   = intent.getStringExtra("download_id") ?: return START_STICKY
                val url  = intent.getStringExtra("url")         ?: return START_STICKY
                val dest = intent.getStringExtra("dest")        ?: return START_STICKY
                resumeDownload(id, url, dest)
                return START_STICKY
            }
            "CANCEL" -> {
                intent.getStringExtra("download_id")?.let { cancelDownload(it) }
                return START_STICKY
            }
        }

        // New download
        val url      = intent?.getStringExtra("url")      ?: return START_NOT_STICKY
        val filename = intent.getStringExtra("filename")  ?: guessFilename(url)
        val notifId  = NOTIF_BASE + startId

        startForeground(notifId, buildProgressNotif(filename, 0))
        scope.launch {
            download(url, filename, notifId)
            stopSelf(startId)
        }
        return START_NOT_STICKY
    }

    // ── Simple download (original path, no pause/resume) ──────────────────────

    private suspend fun download(url: String, filename: String, notifId: Int) {
        val req = Request.Builder().url(url).build()
        runCatching {
            val resp      = http.newCall(req).execute()
            val body      = resp.body ?: throw IOException("empty body")
            val totalBytes = body.contentLength()

            val outDir = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS)
            outDir.mkdirs()
            val file = uniqueFile(outDir, filename)

            var downloaded = 0L
            val buf = ByteArray(8192)
            body.byteStream().use { ins ->
                FileOutputStream(file).use { outs ->
                    while (true) {
                        val n = ins.read(buf)
                        if (n == -1) break
                        outs.write(buf, 0, n)
                        downloaded += n
                        if (totalBytes > 0) {
                            val progress = ((downloaded * 100) / totalBytes).toInt()
                            updateProgressNotif(notifId, filename, progress)
                        }
                    }
                }
            }
            completeNotif(notifId, filename, file)
        }.onFailure { e ->
            errorNotif(notifId, filename, e.message ?: "Unknown error")
        }
    }

    // ── Pause / Resume / Cancel ───────────────────────────────────────────────

    private fun pauseDownload(downloadId: String) {
        pausedDownloads.add(downloadId)
        downloadJobs[downloadId]?.cancel()
        downloadJobs.remove(downloadId)
        statusNotif(downloadId.hashCode(), "Download paused", downloadId)
    }

    private fun resumeDownload(downloadId: String, url: String, dest: String) {
        pausedDownloads.remove(downloadId)
        val resumeFrom = File(dest).takeIf { it.exists() }?.length() ?: 0L
        startResumableJob(downloadId, url, dest, resumeFrom)
    }

    private fun cancelDownload(downloadId: String) {
        cancelledDownloads.add(downloadId)
        downloadJobs[downloadId]?.cancel()
        downloadJobs.remove(downloadId)
        nm.cancel(downloadId.hashCode())
    }

    private fun startResumableJob(downloadId: String, url: String, dest: String, resumeFrom: Long = 0) {
        val job = scope.launch {
            try {
                val req = Request.Builder()
                    .url(url)
                    .apply { if (resumeFrom > 0) header("Range", "bytes=$resumeFrom-") }
                    .build()

                http.newCall(req).execute().use { resp ->
                    val contentLength = resp.body?.contentLength() ?: -1L
                    val total = if (contentLength >= 0) contentLength + resumeFrom else -1L
                    val file  = File(dest)
                    val out   = FileOutputStream(file, resumeFrom > 0)

                    resp.body?.byteStream()?.use { input ->
                        val buf = ByteArray(8192)
                        var written = resumeFrom
                        var read: Int
                        while (input.read(buf).also { read = it } != -1) {
                            if (downloadId in cancelledDownloads) break
                            while (downloadId in pausedDownloads) { delay(500) }
                            out.write(buf, 0, read)
                            written += read
                            if (total > 0) {
                                val progress = ((written * 100) / total).toInt()
                                updateProgressNotif(downloadId.hashCode(), file.name, progress)
                            }
                        }
                        out.flush()
                        out.close()
                    }

                    if (downloadId !in cancelledDownloads) {
                        completeNotif(downloadId.hashCode(), file.name, file)
                    }
                }
            } catch (e: Exception) {
                if (downloadId !in cancelledDownloads) {
                    errorNotif(downloadId.hashCode(), downloadId, e.message ?: "Error")
                }
            }
        }
        downloadJobs[downloadId] = job
    }

    // ── Notifications ─────────────────────────────────────────────────────────

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val ch = NotificationChannel(CHANNEL_ID, "Downloads", NotificationManager.IMPORTANCE_LOW)
            ch.description = "Parsec Browser download progress"
            nm.createNotificationChannel(ch)
        }
    }

    private fun buildProgressNotif(filename: String, progress: Int): Notification =
        NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("Downloading $filename")
            .setProgress(100, progress, progress == 0)
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setOngoing(true)
            .build()

    private fun updateProgressNotif(notifId: Int, filename: String, progress: Int) {
        nm.notify(notifId, buildProgressNotif(filename, progress))
    }

    private fun statusNotif(notifId: Int, title: String, text: String) {
        val notif = NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle(title)
            .setContentText(text)
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setAutoCancel(true)
            .build()
        nm.notify(notifId, notif)
    }

    private fun completeNotif(notifId: Int, filename: String, file: File) {
        val notif = NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("Download complete")
            .setContentText(filename)
            .setSmallIcon(android.R.drawable.stat_sys_download_done)
            .setAutoCancel(true)
            .build()
        nm.notify(notifId, notif)
    }

    private fun errorNotif(notifId: Int, filename: String, error: String) {
        val notif = NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("Download failed")
            .setContentText("$filename: $error")
            .setSmallIcon(android.R.drawable.stat_notify_error)
            .setAutoCancel(true)
            .build()
        nm.notify(notifId, notif)
    }

    // ── Utilities ─────────────────────────────────────────────────────────────

    private fun guessFilename(url: String): String {
        val path = url.substringAfterLast('/').substringBefore('?')
        return path.ifBlank { "download" }
    }

    private fun uniqueFile(dir: File, name: String): File {
        var f       = File(dir, name)
        var counter = 1
        val base    = name.substringBeforeLast('.')
        val ext     = name.substringAfterLast('.', "")
        while (f.exists()) {
            f = if (ext.isNotEmpty()) File(dir, "$base ($counter).$ext")
                else File(dir, "$base ($counter)")
            counter++
        }
        return f
    }

    override fun onDestroy() {
        super.onDestroy()
        scope.cancel()
    }
}
