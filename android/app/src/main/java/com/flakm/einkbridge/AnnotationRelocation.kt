package com.flakm.einkbridge

import org.json.JSONArray
import org.json.JSONObject

internal fun elementMapJson(elements: List<ElementEntry>): String = JSONArray().apply {
    elements.forEach { e -> put(JSONObject().apply {
        put("i", e.i); put("tag", e.tag); put("id", e.id); put("section", e.section)
        // Android 13's JSONObject has put(Double), but no put(Float).
        put("t", e.t.toDouble()); put("b", e.b.toDouble())
        put("l", e.l.toDouble()); put("r", e.r.toDouble()); put("text", e.text)
    }) }
}.toString()

/** What a page load does with the ink it finds, given the saved element map and the one the page reports. */
internal enum class ReconcilePlan { NOTHING, ADOPT_MAP, RELOCATE }

/**
 * An empty saved map means there is nothing to reconcile against, not that every paragraph
 * moved: relocating against it would find no source for any stroke and archive them all.
 */
internal fun reconcilePlan(previous: List<ElementEntry>, current: List<ElementEntry>, bufEmpty: Boolean): ReconcilePlan =
    when {
        current.isEmpty() -> ReconcilePlan.NOTHING
        bufEmpty || previous.isEmpty() || previous == current -> ReconcilePlan.ADOPT_MAP
        else -> ReconcilePlan.RELOCATE
    }

internal data class RelocatedInk(val strokes: List<Stroke>, val groups: List<BindGroup>, val orphanCount: Int)

/** Preserve each annotation's offset from its text, not its absolute page position.
 * Missing/ambiguous targets are archived by the caller rather than drawn over unrelated text. */
internal fun relocateInk(
    strokes: List<Stroke>, groups: List<BindGroup>, previous: List<ElementEntry>,
    current: List<ElementEntry>, targets: List<Int?>,
): RelocatedInk {
    val mapping = previous.mapIndexed { index, old ->
        old.i to targets.getOrNull(index)?.let { i -> current.find { it.i == i } }
    }.toMap()
    val shifted = mutableListOf<Stroke>()
    val indices = mutableMapOf<Int, Int>()
    val deltas = mutableMapOf<Int, Pair<Float, Float>>()
    val free = strokes.filterIndexed { i, _ -> groups.none { i in it.strokeIndices } }
    val sourceByStroke = mutableMapOf<Stroke, ElementEntry>()
    val (clusters, _) = groupStrokesWithProximity(free, previous)
    clusters.forEach { cluster ->
        val refs = (cluster.anchor as? Anchor.Proximity)?.elements.orEmpty()
        val ref = refs.firstOrNull()
        val points = cluster.strokes.flatMap { it.points }
        val cx = points.map { it.first }.average().toFloat()
        val cy = points.map { it.second }.average().toFloat()
        val source = previous.filter { it.tag == ref?.tag && it.text == ref.text }
            .minByOrNull { distToElement(cx, cy, it) }
        if (source != null) cluster.strokes.forEach { sourceByStroke[it] = source }
    }
    strokes.forEachIndexed { index, stroke ->
        if (stroke.points.isEmpty()) return@forEachIndexed
        val group = groups.find { index in it.strokeIndices }
        val old = if (group != null) previous.find { it.i == group.elementIndices.firstOrNull() }
            else sourceByStroke[stroke]
        val target = old?.let { mapping[it.i] }
        if (old == null || target == null) return@forEachIndexed
        // Top-left maintains margin notes when paragraphs wrap or grow.
        val dx = target.l - old.l
        val dy = target.t - old.t
        indices[index] = shifted.size
        shifted.add(stroke.copy(points = stroke.points.map { (x, y) -> x + dx to y + dy }))
        group?.let { deltas[it.id] = dx to dy }
    }
    val movedGroups = groups.mapNotNull { group ->
        val newIndices = group.strokeIndices.mapNotNull { indices[it] }.toSet()
        val elements = group.elementIndices.mapNotNull { mapping[it] }
        val delta = deltas[group.id]
        if (newIndices.isEmpty() || elements.isEmpty() || delta == null) null
        else group.copy(strokeIndices = newIndices, elementIndices = elements.map { it.i },
            elementRefs = elements.map { ElementRef(it.section, it.tag, it.text) },
            markerDocX = group.markerDocX + delta.first, markerDocY = group.markerDocY + delta.second,
            strokeDocCenters = group.strokeDocCenters.map { (x, y) -> x + delta.first to y + delta.second },
            elementDocCenters = elements.map { (it.l + it.r) / 2 to (it.t + it.b) / 2 })
    }
    return RelocatedInk(shifted, movedGroups, strokes.size - shifted.size)
}
