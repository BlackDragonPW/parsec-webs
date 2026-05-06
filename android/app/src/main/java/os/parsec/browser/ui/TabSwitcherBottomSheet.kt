package os.parsec.browser.ui

import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.os.Bundle
import android.view.*
import android.widget.*
import androidx.recyclerview.widget.*
import com.google.android.material.bottomsheet.BottomSheetDialogFragment

/**
 * TabSwitcherBottomSheet — shows all open tabs with favicon, title, URL and close button.
 * Fixed: replaced crash-prone android.R.layout.simple_list_item_2 (missing android.R.id.icon)
 *        with a fully programmatic card layout that supports close gestures and active-tab highlight.
 */
class TabSwitcherBottomSheet(
    private val tabs: List<BrowserActivity.TabEntry>,
    private val activeTabId: String?,
    private val onTabSelected: (String) -> Unit,
    private val onTabClosed: (String) -> Unit,
    private val onNewTab: () -> Unit,
) : BottomSheetDialogFragment() {

    override fun onCreateView(
        inflater: LayoutInflater, container: ViewGroup?, savedInstanceState: Bundle?
    ): View {
        val ctx = requireContext()
        val root = LinearLayout(ctx).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(0xFF0f172a.toInt())
            setPadding(0, 0, 0, 48)
        }

        // ── Header ──────────────────────────────────────────────────────────
        val header = LinearLayout(ctx).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(48, 32, 32, 20)
        }
        val titleTv = TextView(ctx).apply {
            text = "${tabs.size} Open Tabs"
            textSize = 18f
            setTypeface(null, Typeface.BOLD)
            setTextColor(0xFFe2e8f0.toInt())
            layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f)
        }
        val btnNew = Button(ctx).apply {
            text = "+ New Tab"
            textSize = 13f
            setTextColor(0xFF818cf8.toInt())
            background = null
            setOnClickListener { onNewTab(); dismiss() }
        }
        header.addView(titleTv)
        header.addView(btnNew)
        root.addView(header)

        // ── Divider ─────────────────────────────────────────────────────────
        root.addView(View(ctx).apply {
            layoutParams = LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 1).also {
                it.bottomMargin = 8
            }
            setBackgroundColor(0xFF1e293b.toInt())
        })

        // ── Tab list ────────────────────────────────────────────────────────
        val rv = RecyclerView(ctx).apply {
            layoutManager = LinearLayoutManager(ctx)
            adapter = TabAdapter()
            // Swipe-to-close gesture
            val swipe = object : ItemTouchHelper.SimpleCallback(0, ItemTouchHelper.LEFT or ItemTouchHelper.RIGHT) {
                override fun onMove(rv: RecyclerView, vh: RecyclerView.ViewHolder, t: RecyclerView.ViewHolder) = false
                override fun onSwiped(vh: RecyclerView.ViewHolder, dir: Int) {
                    val tab = tabs[vh.adapterPosition]
                    onTabClosed(tab.id)
                    dismiss()
                }
            }
            ItemTouchHelper(swipe).attachToRecyclerView(this)
        }
        root.addView(rv, LinearLayout.LayoutParams(
            LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f
        ))

        return root
    }

    inner class TabAdapter : RecyclerView.Adapter<TabAdapter.VH>() {

        inner class VH(val card: LinearLayout) : RecyclerView.ViewHolder(card) {
            val favicon: TextView
            val title:   TextView
            val url:     TextView
            val close:   TextView
            val incogBadge: TextView

            init {
                val ctx = card.context
                card.orientation  = LinearLayout.HORIZONTAL
                card.gravity      = Gravity.CENTER_VERTICAL
                card.setPadding(32, 20, 24, 20)

                // Left: favicon
                favicon = TextView(ctx).apply {
                    textSize  = 20f
                    minWidth  = 80
                    gravity   = Gravity.CENTER
                }
                card.addView(favicon)

                // Center: title + url stack
                val info = LinearLayout(ctx).apply {
                    orientation = LinearLayout.VERTICAL
                    layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f).also {
                        it.marginStart = 16
                        it.marginEnd   = 8
                    }
                }
                // Row: title + optional incognito badge
                val titleRow = LinearLayout(ctx).apply {
                    orientation = LinearLayout.HORIZONTAL
                    gravity     = Gravity.CENTER_VERTICAL
                }
                title = TextView(ctx).apply {
                    textSize  = 14f
                    setTypeface(null, Typeface.BOLD)
                    setTextColor(0xFFe2e8f0.toInt())
                    maxLines  = 1
                    ellipsize = android.text.TextUtils.TruncateAt.END
                    layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f)
                }
                incogBadge = TextView(ctx).apply {
                    text    = "🕵"
                    textSize = 11f
                    visibility = View.GONE
                    setPadding(6, 2, 6, 2)
                }
                titleRow.addView(title)
                titleRow.addView(incogBadge)
                info.addView(titleRow)

                url = TextView(ctx).apply {
                    textSize  = 11f
                    setTextColor(0xFF64748b.toInt())
                    maxLines  = 1
                    ellipsize = android.text.TextUtils.TruncateAt.MIDDLE
                }
                info.addView(url)
                card.addView(info)

                // Right: close button
                close = TextView(ctx).apply {
                    text      = "✕"
                    textSize  = 16f
                    setTextColor(0xFF475569.toInt())
                    setPadding(16, 8, 8, 8)
                }
                card.addView(close)
            }
        }

        override fun onCreateViewHolder(parent: ViewGroup, viewType: Int): VH {
            val ctx  = parent.context
            val card = LinearLayout(ctx)

            // Ripple-like touch feedback via background selector
            val bg = GradientDrawable().apply {
                shape         = GradientDrawable.RECTANGLE
                cornerRadius  = 12f * ctx.resources.displayMetrics.density
                setColor(0x00000000)
            }
            card.background = bg
            card.layoutParams = RecyclerView.LayoutParams(
                RecyclerView.LayoutParams.MATCH_PARENT,
                RecyclerView.LayoutParams.WRAP_CONTENT
            ).also { it.bottomMargin = 4 }

            return VH(card)
        }

        override fun onBindViewHolder(holder: VH, position: Int) {
            val tab  = tabs[position]
            val active = tab.id == activeTabId

            holder.favicon.text    = tab.favicon.ifEmpty { "🌐" }
            holder.title.text      = tab.title.ifEmpty { "New Tab" }
            holder.url.text        = tab.url
            holder.incogBadge.visibility = if (tab.incognito) View.VISIBLE else View.GONE

            // Highlight active tab
            if (active) {
                holder.card.setBackgroundColor(0x22818cf8.toInt())
                holder.title.setTextColor(0xFF818cf8.toInt())
            } else {
                holder.card.setBackgroundColor(Color.TRANSPARENT)
                holder.title.setTextColor(0xFFe2e8f0.toInt())
            }

            holder.itemView.setOnClickListener {
                onTabSelected(tab.id)
                dismiss()
            }

            holder.close.setOnClickListener {
                onTabClosed(tab.id)
                dismiss()
            }
        }

        override fun getItemCount() = tabs.size
    }
}
