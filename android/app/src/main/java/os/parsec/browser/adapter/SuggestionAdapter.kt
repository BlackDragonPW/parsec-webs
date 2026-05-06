package os.parsec.browser.adapter

import android.view.Gravity
import android.view.ViewGroup
import android.widget.LinearLayout
import android.widget.TextView
import androidx.recyclerview.widget.RecyclerView
import com.google.gson.JsonArray

/**
 * SuggestionAdapter — address bar autocomplete.
 *
 * FIX: android.R.layout.simple_list_item_2 has no android.R.id.icon child —
 *      replaced with a fully programmatic 3-element layout (favicon + title + url).
 */
class SuggestionAdapter(
    private val items: JsonArray,
    private val onSelected: (String) -> Unit
) : RecyclerView.Adapter<SuggestionAdapter.ViewHolder>() {

    class ViewHolder(val row: LinearLayout) : RecyclerView.ViewHolder(row) {
        val favicon:  TextView
        val title:    TextView
        val subtitle: TextView

        init {
            val ctx = row.context
            row.orientation = LinearLayout.HORIZONTAL
            row.gravity     = Gravity.CENTER_VERTICAL
            row.setPadding(32, 20, 32, 20)

            favicon = TextView(ctx).apply {
                textSize = 18f
                minWidth = 72
                gravity  = Gravity.CENTER
            }
            row.addView(favicon)

            val textCol = LinearLayout(ctx).apply {
                orientation  = LinearLayout.VERTICAL
                layoutParams = LinearLayout.LayoutParams(
                    0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f
                ).also { it.marginStart = 16 }
            }
            title = TextView(ctx).apply {
                textSize  = 14f
                setTextColor(0xFFe2e8f0.toInt())
                maxLines  = 1
                ellipsize = android.text.TextUtils.TruncateAt.END
            }
            subtitle = TextView(ctx).apply {
                textSize  = 11f
                setTextColor(0xFF64748b.toInt())
                maxLines  = 1
                ellipsize = android.text.TextUtils.TruncateAt.MIDDLE
            }
            textCol.addView(title)
            textCol.addView(subtitle)
            row.addView(textCol)
        }
    }

    override fun onCreateViewHolder(parent: ViewGroup, viewType: Int): ViewHolder {
        val row = LinearLayout(parent.context).apply {
            layoutParams = RecyclerView.LayoutParams(
                RecyclerView.LayoutParams.MATCH_PARENT,
                RecyclerView.LayoutParams.WRAP_CONTENT
            )
            setBackgroundColor(0xFF1e293b.toInt())
            isClickable = true
            isFocusable = true
        }
        return ViewHolder(row)
    }

    override fun onBindViewHolder(holder: ViewHolder, position: Int) {
        val item    = items.get(position).asJsonObject
        val type    = item.get("type")?.asString   ?: "search"
        val url     = item.get("url")?.asString    ?: ""
        val title   = item.get("title")?.asString  ?: url
        val favicon = item.get("favicon")?.asString ?: when (type) {
            "history"  -> "🕐"
            "bookmark" -> "⭐"
            else       -> "🔍"
        }
        holder.favicon.text  = favicon
        holder.title.text    = title
        holder.subtitle.text = url
        holder.itemView.setOnClickListener { onSelected(url) }
    }

    override fun getItemCount(): Int = items.size()
}
