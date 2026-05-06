package os.parsec.browser.ui

import android.content.Context
import android.os.Bundle
import android.view.*
import android.widget.*
import com.google.android.material.bottomsheet.BottomSheetDialogFragment

/**
 * MenuBottomSheet — the ⋮ overflow menu for the browser.
 * Presents actions: New Tab, Incognito, Bookmark, Share, Desktop Mode,
 * Downloads, History, Bookmarks, Settings, Find in Page, Zoom.
 */
class MenuBottomSheet(
    private val activity: BrowserActivity,
    private val currentUrl: String,
    private val onNewTab: () -> Unit,
    private val onIncognito: () -> Unit,
    private val onBookmark: () -> Unit,
    private val onShare: () -> Unit,
    private val onDesktop: () -> Unit,
    private val onDownloads: () -> Unit,
    private val onHistory: () -> Unit,
    private val onBookmarks: () -> Unit,
    private val onSettings: () -> Unit,
    private val onFindInPage: () -> Unit,
    private val onZoomIn: () -> Unit,
    private val onZoomOut: () -> Unit,
) : BottomSheetDialogFragment() {

    override fun onCreateView(
        inflater: LayoutInflater, container: ViewGroup?, savedInstanceState: Bundle?
    ): View {
        val root = LinearLayout(requireContext()).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(16, 16, 16, 32)
        }

        fun menuItem(icon: String, label: String, action: () -> Unit) {
            val row = LinearLayout(requireContext()).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = Gravity.CENTER_VERTICAL
                minimumHeight = 128
                isClickable = true
                isFocusable = true
                setOnClickListener { action(); dismiss() }
                setPadding(32, 8, 32, 8)
            }
            val ico = TextView(requireContext()).apply {
                text = icon; textSize = 20f
                layoutParams = LinearLayout.LayoutParams(96, LinearLayout.LayoutParams.WRAP_CONTENT)
            }
            val lbl = TextView(requireContext()).apply {
                text = label; textSize = 16f
            }
            row.addView(ico)
            row.addView(lbl)
            root.addView(row)
        }

        menuItem("➕", "New Tab")       { onNewTab() }
        menuItem("🕵️", "New Incognito") { onIncognito() }
        menuItem("⭐", "Bookmark page") { onBookmark() }
        menuItem("🔗", "Share URL")      { onShare() }
        menuItem("🖥️", "Desktop mode")  { onDesktop() }
        menuItem("🔍", "Find in page")  { onFindInPage() }

        // Zoom row
        val zoomRow = LinearLayout(requireContext()).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(32, 8, 32, 8)
        }
        val zoomLbl = TextView(requireContext()).apply {
            text = "🔎 Zoom"; textSize = 16f
            layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f)
        }
        val zoomOut = Button(requireContext()).apply { text = "−"; setOnClickListener { onZoomOut() } }
        val zoomIn  = Button(requireContext()).apply { text = "+"; setOnClickListener { onZoomIn() } }
        zoomRow.addView(zoomLbl); zoomRow.addView(zoomOut); zoomRow.addView(zoomIn)
        root.addView(zoomRow)

        menuItem("⬇️", "Downloads")  { onDownloads() }
        menuItem("🕐", "History")     { onHistory() }
        menuItem("📚", "Bookmarks")  { onBookmarks() }
        menuItem("⚙️", "Settings")   { onSettings() }

        return root
    }
}
