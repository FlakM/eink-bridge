package com.flakm.einkbridge

import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.TextView
import androidx.recyclerview.widget.DiffUtil
import androidx.recyclerview.widget.ListAdapter
import androidx.recyclerview.widget.RecyclerView
import java.time.Duration
import java.time.Instant
import java.time.LocalDateTime
import java.time.ZoneId
import java.time.format.DateTimeFormatter

internal fun statusIcon(status: String): String = when (status) {
    "Active" -> "\u25CF"     // ● filled circle
    "Submitted" -> "\u2713"  // ✓ check
    else -> "\u25CB"         // ○ empty circle
}

internal fun formatSessionTime(iso: String, now: LocalDateTime = LocalDateTime.now()): String {
    return try {
        val instant = Instant.parse(iso)
        val local = LocalDateTime.ofInstant(instant, ZoneId.systemDefault())
        val diff = Duration.between(local, now)
        when {
            diff.toMinutes() < 1 -> "just now"
            diff.toMinutes() < 60 -> "${diff.toMinutes()}m ago"
            diff.toHours() < 24 -> "${diff.toHours()}h ago"
            else -> local.format(DateTimeFormatter.ofPattern("MMM d, HH:mm"))
        }
    } catch (_: Exception) {
        iso.take(16)
    }
}

class SessionAdapter(
    private val onClick: (SessionInfo) -> Unit
) : ListAdapter<SessionInfo, SessionAdapter.ViewHolder>(DIFF) {

    private var pendingStrokeIds: Set<String> = emptySet()
    private var cachedSessionIds: Set<String> = emptySet()

    fun setPendingStrokes(ids: Set<String>) {
        if (ids != pendingStrokeIds) {
            pendingStrokeIds = ids
            notifyDataSetChanged()
        }
    }

    fun setCachedSessions(ids: Set<String>) {
        if (ids != cachedSessionIds) {
            cachedSessionIds = ids
            notifyDataSetChanged()
        }
    }

    class ViewHolder(view: View) : RecyclerView.ViewHolder(view) {
        val title: TextView = view.findViewById(android.R.id.text1)
        val subtitle: TextView = view.findViewById(android.R.id.text2)
    }

    override fun onCreateViewHolder(parent: ViewGroup, viewType: Int): ViewHolder {
        val view = LayoutInflater.from(parent.context).inflate(R.layout.item_session, parent, false)
        return ViewHolder(view)
    }

    override fun onBindViewHolder(holder: ViewHolder, position: Int) {
        val session = getItem(position)
        val hasPending = session.id in pendingStrokeIds
        val isCached = session.id in cachedSessionIds
        val icon = statusIcon(session.status)
        val pendingMark = if (hasPending) "  \u270E" else ""
        val cachedMark = if (isCached) "  \u2B07" else ""
        holder.title.text = "$icon  ${session.title}$pendingMark$cachedMark"
        val statusParts = mutableListOf(session.status)
        if (hasPending) statusParts.add("unsaved strokes")
        if (isCached) statusParts.add("cached")
        holder.subtitle.text = "${statusParts.joinToString(" \u2014 ")} \u2014 ${formatSessionTime(session.updatedAt)}"
        holder.itemView.setOnClickListener { onClick(session) }
    }

    companion object {
        val DIFF = object : DiffUtil.ItemCallback<SessionInfo>() {
            override fun areItemsTheSame(a: SessionInfo, b: SessionInfo) = a.id == b.id
            override fun areContentsTheSame(a: SessionInfo, b: SessionInfo) = a == b
        }
    }
}
