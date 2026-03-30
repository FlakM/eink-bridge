package com.flakm.einkbridge

import android.graphics.Color
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.LinearLayout
import android.widget.TextView
import androidx.recyclerview.widget.DiffUtil
import androidx.recyclerview.widget.ListAdapter
import androidx.recyclerview.widget.RecyclerView

class SessionAdapter(
    private val onClick: (SessionInfo) -> Unit
) : ListAdapter<SessionInfo, SessionAdapter.ViewHolder>(DIFF) {

    class ViewHolder(view: View) : RecyclerView.ViewHolder(view) {
        val title: TextView = view.findViewById(android.R.id.text1)
        val subtitle: TextView = view.findViewById(android.R.id.text2)
    }

    override fun onCreateViewHolder(parent: ViewGroup, viewType: Int): ViewHolder {
        val layout = LinearLayout(parent.context).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(0, 24, 0, 24)
            layoutParams = ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT
            )
        }
        val title = TextView(parent.context).apply {
            id = android.R.id.text1
            textSize = 18f
            setTextColor(Color.BLACK)
        }
        val subtitle = TextView(parent.context).apply {
            id = android.R.id.text2
            textSize = 14f
            setTextColor(Color.DKGRAY)
        }
        layout.addView(title)
        layout.addView(subtitle)
        return ViewHolder(layout)
    }

    override fun onBindViewHolder(holder: ViewHolder, position: Int) {
        val session = getItem(position)
        val icon = when (session.status) {
            "Active" -> "\u25CF"
            "Submitted" -> "\u2713"
            else -> "\u25CB"
        }
        holder.title.text = "$icon  ${session.title}"
        holder.subtitle.text = "${session.status} \u2014 ${session.id}"
        holder.itemView.setOnClickListener { onClick(session) }
    }

    companion object {
        val DIFF = object : DiffUtil.ItemCallback<SessionInfo>() {
            override fun areItemsTheSame(a: SessionInfo, b: SessionInfo) = a.id == b.id
            override fun areContentsTheSame(a: SessionInfo, b: SessionInfo) = a == b
        }
    }
}
