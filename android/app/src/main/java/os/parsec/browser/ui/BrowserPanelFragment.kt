package os.parsec.browser.ui

import android.os.Bundle
import android.view.*
import android.widget.*
import androidx.appcompat.widget.SwitchCompat
import androidx.recyclerview.widget.*
import com.google.android.material.bottomsheet.BottomSheetDialogFragment
import com.google.gson.Gson
import com.google.gson.JsonArray
import com.google.gson.JsonObject
import kotlinx.coroutines.*
import os.parsec.browser.ParsecCore
import os.parsec.browser.ResourceBlocker

/**
 * BrowserPanelFragment — bottom sheet panel for:
 *   "history", "bookmarks", "downloads", "settings"
 *
 * Usage:
 *   BrowserPanelFragment.newInstance("history") { url -> navigateTab(activeTabId, url) }
 */
class BrowserPanelFragment : BottomSheetDialogFragment() {

    private var panel: String = "history"
    private var onNavigate: ((String) -> Unit)? = null
    private val gson = Gson()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)

    companion object {
        fun newInstance(panel: String, onNavigate: (String) -> Unit): BrowserPanelFragment {
            return BrowserPanelFragment().also {
                it.panel = panel
                it.onNavigate = onNavigate
            }
        }
    }

    override fun onCreateView(
        inflater: LayoutInflater, container: ViewGroup?, savedInstanceState: Bundle?
    ): View {
        val root = LinearLayout(requireContext()).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(0, 0, 0, 32)
        }

        val titleText = when (panel) {
            "history"   -> "🕐 History"
            "bookmarks" -> "📚 Bookmarks"
            "downloads" -> "⬇️ Downloads"
            "settings"  -> "⚙️ Settings"
            else        -> panel.replaceFirstChar { it.uppercase() }
        }

        val header = TextView(requireContext()).apply {
            text = titleText
            textSize = 20f
            setTypeface(null, android.graphics.Typeface.BOLD)
            setPadding(32, 24, 32, 8)
        }
        root.addView(header)

        when (panel) {
            "settings" -> root.addView(buildSettingsView())
            else       -> root.addView(buildListView())
        }
        return root
    }

    // ── List panel (history / bookmarks / downloads) ──────────────────────────

    private fun buildListView(): View {
        val rv = RecyclerView(requireContext()).apply {
            layoutManager = LinearLayoutManager(requireContext())
        }
        scope.launch {
            val data = withContext(Dispatchers.IO) { loadData() }
            rv.adapter = PanelListAdapter(data)
        }
        return rv
    }

    private fun loadData(): JsonArray {
        val cmd = when (panel) {
            "history"   -> "GetHistory"
            "bookmarks" -> "GetBookmarks"
            "downloads" -> "GetDownloads"
            else        -> return JsonArray()
        }
        val resp = ParsecCore.ipc("""{"id":"panel","cmd":"$cmd","args":{}}""")
        return runCatching {
            gson.fromJson(resp, JsonObject::class.java)
                .get("data")?.asJsonArray ?: JsonArray()
        }.getOrDefault(JsonArray())
    }

    inner class PanelListAdapter(private val items: JsonArray)
        : RecyclerView.Adapter<PanelListAdapter.VH>() {

        inner class VH(view: View) : RecyclerView.ViewHolder(view) {
            val title:    TextView = view.findViewById(android.R.id.text1)
            val subtitle: TextView = view.findViewById(android.R.id.text2)
        }

        override fun onCreateViewHolder(parent: ViewGroup, viewType: Int): VH {
            val v = LayoutInflater.from(parent.context)
                .inflate(android.R.layout.simple_list_item_2, parent, false)
            return VH(v)
        }

        override fun onBindViewHolder(holder: VH, position: Int) {
            val item = items.get(position).asJsonObject
            val title = item.get("title")?.asString
                ?: item.get("filename")?.asString
                ?: item.get("url")?.asString ?: ""
            val url   = item.get("url")?.asString ?: ""
            holder.title.text    = title
            holder.subtitle.text = url
            holder.itemView.setOnClickListener {
                if (url.isNotBlank()) {
                    onNavigate?.invoke(url)
                    dismiss()
                }
            }
        }

        override fun getItemCount() = items.size()
    }

    // ── Settings panel ────────────────────────────────────────────────────────

    private fun buildSettingsView(): View {
        val scroll = ScrollView(requireContext())
        val container = LinearLayout(requireContext()).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(32, 8, 32, 32)
        }

        fun switchPref(label: String, prefKey: String, default: Boolean) {
            // Load current pref
            val resp = runCatching {
                ParsecCore.ipc("""{"id":"p","cmd":"GetPrefs","args":{}}""")
            }.getOrDefault("{}")
            val curVal = runCatching {
                gson.fromJson(resp, JsonObject::class.java)
                    .get("data")?.asJsonObject?.get(prefKey)?.asBoolean ?: default
            }.getOrDefault(default)

            val row = LinearLayout(requireContext()).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = Gravity.CENTER_VERTICAL
                minimumHeight = 120
                setPadding(0, 8, 0, 8)
            }
            val lbl = TextView(requireContext()).apply {
                text = label; textSize = 16f
                layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f)
            }
            val sw = SwitchCompat(requireContext()).apply {
                isChecked = curVal
                setOnCheckedChangeListener { _, checked ->
                    ParsecCore.ipc("""{"id":"s","cmd":"SetPref","args":{"key":"$prefKey","value":$checked}}""")
                    when (prefKey) {
                        "block_ads", "block_trackers", "block_nsfw", "https_only" -> {
                            ResourceBlocker.refreshPrefs(
                                com.google.gson.JsonParser.parseString(
                                    ParsecCore.ipc("""{"id":"0","cmd":"GetPrefs","args":{}}""")
                                ).asJsonObject.getAsJsonObject("data")
                            )
                        }
                    }
                }
            }
            row.addView(lbl); row.addView(sw)
            container.addView(row)
        }

        switchPref("Block Ads",      "block_ads",      true)
        switchPref("Block Trackers", "block_trackers", true)
        switchPref("HTTPS Only",     "https_only",     true)
        switchPref("Block Popups",   "block_popups",   true)
        switchPref("Do Not Track",   "do_not_track",   true)
        switchPref("Desktop Mode",   "desktop_mode",   false)
        switchPref("Save Data",      "save_data",      false)
        switchPref("Tab Suspend",    "auto_suspend_tabs", true)
        switchPref("Ghost Mode",     "ghost_mode",       false)

        scroll.addView(container)
        return scroll
    }

    override fun onDestroyView() {
        super.onDestroyView()
        scope.cancel()
    }
}
