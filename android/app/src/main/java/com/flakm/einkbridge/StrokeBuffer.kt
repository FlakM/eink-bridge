package com.flakm.einkbridge

import android.graphics.Color
import org.json.JSONArray
import org.json.JSONObject
import kotlin.math.max
import kotlin.math.min
import kotlin.math.sqrt

internal data class Stroke(val points: List<Pair<Float, Float>>, val width: Float, val color: Int = Color.BLACK)

internal data class ElementEntry(
    val i: Int,
    val tag: String,
    val id: String?,
    val section: String?,
    val t: Float,
    val b: Float,
    val l: Float,
    val r: Float,
    val text: String,
)

internal data class ElementRef(
    val sectionId: String?,
    val tag: String,
    val text: String,
)

internal sealed class Anchor {
    data class Proximity(val elements: List<ElementRef>) : Anchor()
    data class Explicit(val elements: List<ElementRef>) : Anchor()
}

internal data class StrokeGroup(
    val anchor: Anchor?,
    val strokes: List<Stroke>,
)

internal data class FoundElement(val i: Int, val tag: String, val id: String?, val section: String?, val text: String, val cx: Float = 0f, val cy: Float = 0f)

internal data class BindGroup(
    val id: Int,
    val color: Int,
    val strokeIndices: Set<Int>,
    val elementIndices: List<Int>,
    val elementRefs: List<ElementRef>,
    val markerDocX: Float,
    val markerDocY: Float,
    val strokeDocCenters: List<Pair<Float, Float>> = emptyList(),
    val elementDocCenters: List<Pair<Float, Float>> = emptyList(),
)

/**
 * WebView transform state used to convert between screen and document coordinates.
 * Screen → Document: docX = (screenX + scrollX) / scale
 * Document → Screen: screenX = docX * scale - scrollX
 */
internal data class ViewTransform(
    val scrollX: Float = 0f,
    val scrollY: Float = 0f,
    val scale: Float = 1f,
) {
    fun screenToDocX(sx: Float) = (sx + scrollX) / scale
    fun screenToDocY(sy: Float) = (sy + scrollY) / scale
    fun docToScreenX(dx: Float) = dx * scale - scrollX
    fun docToScreenY(dy: Float) = dy * scale - scrollY
    fun docToScreenWidth(w: Float) = w * scale
}

/**
 * Pure-Kotlin stroke accumulator — no Android dependencies, fully unit-testable.
 *
 * All stored coordinates are in **document space**. Callers pass a [ViewTransform]
 * to [begin]/[addPoint]/[end] so screen-space SDK input is converted on the fly.
 */
internal class StrokeBuffer {
    private val _strokes = mutableListOf<Stroke>()
    private var currentPoints = mutableListOf<Pair<Float, Float>>()
    private var currentWidth = 3f
    private var currentColor: Int = Color.BLACK
    private var currentTransform = ViewTransform()

    val strokes: List<Stroke> get() = _strokes.toList()
    val isEmpty: Boolean get() = _strokes.isEmpty() && currentPoints.size < 2
    val size: Int get() = _strokes.size

    fun setColor(color: Int) { currentColor = color }

    /** Returns committed strokes plus the in-progress stroke (if any). */
    fun allStrokes(): List<Stroke> {
        if (currentPoints.size < 2) return _strokes.toList()
        return _strokes + Stroke(currentPoints.toList(), currentWidth, currentColor)
    }

    fun begin(x: Float, y: Float, width: Float = 3f, transform: ViewTransform = ViewTransform()) {
        currentTransform = transform
        currentWidth = width / transform.scale
        val docPt = transform.screenToDocX(x) to transform.screenToDocY(y)
        currentPoints = mutableListOf(docPt)
    }

    fun addPoint(x: Float, y: Float) {
        currentPoints.add(currentTransform.screenToDocX(x) to currentTransform.screenToDocY(y))
    }

    fun end(x: Float, y: Float) {
        val docPt = currentTransform.screenToDocX(x) to currentTransform.screenToDocY(y)
        currentPoints.add(docPt)
        commit()
    }

    fun commit() {
        if (currentPoints.size > 1) {
            _strokes.add(Stroke(currentPoints.toList(), currentWidth, currentColor))
        }
        currentPoints = mutableListOf()
    }

    fun undo() {
        if (_strokes.isNotEmpty()) _strokes.removeAt(_strokes.lastIndex)
    }

    fun clear() {
        _strokes.clear()
        currentPoints.clear()
    }

    /**
     * Removes every committed stroke that comes within [radius] screen-pixels of any point in [path].
     * [path] is in screen coordinates; stored strokes are in document coordinates.
     */
    fun erase(path: List<Pair<Float, Float>>, radius: Float, transform: ViewTransform = ViewTransform()): Boolean {
        if (path.isEmpty()) return false
        val docPath = path.map { (x, y) -> transform.screenToDocX(x) to transform.screenToDocY(y) }
        val docRadius = radius / transform.scale
        val r2 = docRadius * docRadius
        val before = _strokes.size
        _strokes.removeAll { stroke ->
            stroke.points.any { (sx, sy) ->
                docPath.any { (ex, ey) ->
                    (ex - sx) * (ex - sx) + (ey - sy) * (ey - sy) <= r2
                }
            }
        }
        return _strokes.size < before
    }

    fun toJson(): String {
        val arr = JSONArray()
        for (stroke in _strokes) {
            val obj = JSONObject()
            obj.put("w", stroke.width.toDouble())
            if (stroke.color != Color.BLACK) obj.put("c", stroke.color)
            val pts = JSONArray()
            for ((x, y) in stroke.points) {
                pts.put(JSONArray().apply { put(x.toDouble()); put(y.toDouble()) })
            }
            obj.put("pts", pts)
            arr.put(obj)
        }
        return arr.toString()
    }

    fun loadJson(json: String) {
        _strokes.clear()
        currentPoints.clear()
        val arr = JSONArray(json)
        for (i in 0 until arr.length()) {
            val obj = arr.getJSONObject(i)
            val w = obj.getDouble("w").toFloat()
            val color = if (obj.has("c")) obj.getInt("c") else Color.BLACK
            val pts = obj.getJSONArray("pts")
            val points = mutableListOf<Pair<Float, Float>>()
            for (j in 0 until pts.length()) {
                val p = pts.getJSONArray(j)
                points.add(p.getDouble(0).toFloat() to p.getDouble(1).toFloat())
            }
            if (points.size > 1) _strokes.add(Stroke(points, w, color))
        }
    }
}

internal fun parseElementMap(json: String): List<ElementEntry> {
    val arr = JSONArray(json)
    return (0 until arr.length()).map { i ->
        val o = arr.getJSONObject(i)
        ElementEntry(
            i = o.getInt("i"),
            tag = o.getString("tag"),
            id = o.optString("id", null),
            section = o.optString("section", null),
            t = o.getDouble("t").toFloat(),
            b = o.getDouble("b").toFloat(),
            l = o.getDouble("l").toFloat(),
            r = o.getDouble("r").toFloat(),
            text = o.optString("text", ""),
        )
    }
}

internal fun strokeCentroid(stroke: Stroke): Pair<Float, Float> {
    val cx = stroke.points.map { it.first }.average().toFloat()
    val cy = stroke.points.map { it.second }.average().toFloat()
    return cx to cy
}

internal fun distToElement(px: Float, py: Float, el: ElementEntry): Float {
    val dx = max(el.l - px, max(0f, px - el.r))
    val dy = max(el.t - py, max(0f, py - el.b))
    return sqrt(dx * dx + dy * dy)
}

internal fun elementToRef(el: ElementEntry): ElementRef =
    ElementRef(sectionId = el.section ?: el.id, tag = el.tag, text = el.text)

internal fun groupStrokesWithProximity(
    strokes: List<Stroke>,
    elements: List<ElementEntry>,
    explicitBindings: Map<Int, List<ElementEntry>> = emptyMap(),
    clusterThreshold: Float = 120f,
): Pair<List<StrokeGroup>, List<Stroke>> {
    if (elements.isEmpty()) return emptyList<StrokeGroup>() to strokes

    // Separate explicitly bound strokes from free ones
    val explicitGrouped = mutableMapOf<Int, MutableList<Stroke>>()
    val free = mutableListOf<Stroke>()
    for ((idx, stroke) in strokes.withIndex()) {
        val els = explicitBindings[idx]
        if (els != null) {
            for (el in els) explicitGrouped.getOrPut(el.i) { mutableListOf() }.add(stroke)
        } else {
            free.add(stroke)
        }
    }

    // Cluster free strokes by centroid proximity (union-find)
    val n = free.size
    val centroids = free.map { strokeCentroid(it) }
    val parent = IntArray(n) { it }

    fun find(x: Int): Int {
        if (parent[x] != x) parent[x] = find(parent[x])
        return parent[x]
    }

    for (i in 0 until n) {
        for (j in i + 1 until n) {
            val dx = centroids[i].first - centroids[j].first
            val dy = centroids[i].second - centroids[j].second
            if (dx * dx + dy * dy <= clusterThreshold * clusterThreshold) {
                val pi = find(i); val pj = find(j)
                if (pi != pj) parent[pi] = pj
            }
        }
    }

    val clusterMap = mutableMapOf<Int, MutableList<Int>>()
    for (i in 0 until n) clusterMap.getOrPut(find(i)) { mutableListOf() }.add(i)

    val proxGrouped = mutableMapOf<Int, MutableList<Stroke>>()
    for ((_, indices) in clusterMap) {
        val clusterStrokes = indices.map { free[it] }
        val pts = clusterStrokes.flatMap { it.points }
        val cx = pts.map { it.first }.average().toFloat()
        val cy = pts.map { it.second }.average().toFloat()
        val nearest = elements.minByOrNull { distToElement(cx, cy, it) }!!
        proxGrouped.getOrPut(nearest.i) { mutableListOf() }.addAll(clusterStrokes)
    }

    val result = mutableListOf<StrokeGroup>()
    for ((elIdx, strks) in explicitGrouped) {
        result.add(StrokeGroup(Anchor.Explicit(listOf(elementToRef(elements.first { it.i == elIdx }))), strks))
    }
    for ((elIdx, strks) in proxGrouped) {
        result.add(StrokeGroup(Anchor.Proximity(listOf(elementToRef(elements.first { it.i == elIdx }))), strks))
    }
    return result to emptyList()
}

internal fun annotationsToJson(groups: List<StrokeGroup>, unanchored: List<Stroke>): String {
    val arr = JSONArray()
    for (group in groups) {
        val obj = JSONObject()
        group.anchor?.let { anchor ->
            val anchorObj = JSONObject()
            when (anchor) {
                is Anchor.Proximity -> {
                    anchorObj.put("type", "proximity")
                    anchorObj.put("elements", elementsToJson(anchor.elements))
                }
                is Anchor.Explicit -> {
                    anchorObj.put("type", "explicit")
                    anchorObj.put("elements", elementsToJson(anchor.elements))
                }
            }
            obj.put("anchor", anchorObj)
        }
        obj.put("strokes", strokesToPointArrays(group.strokes))
        arr.put(obj)
    }
    if (unanchored.isNotEmpty()) {
        val obj = JSONObject()
        obj.put("strokes", strokesToPointArrays(unanchored))
        arr.put(obj)
    }
    return arr.toString()
}

internal fun bindGroupsToAnnotations(
    strokes: List<Stroke>,
    bindGroups: List<BindGroup>,
): Pair<List<StrokeGroup>, List<Stroke>> {
    val usedIndices = bindGroups.flatMap { it.strokeIndices }.toSet()
    val groups = bindGroups.mapNotNull { bg ->
        val groupStrokes = bg.strokeIndices.mapNotNull { strokes.getOrNull(it) }
        if (groupStrokes.isEmpty() && bg.elementRefs.isEmpty()) return@mapNotNull null
        StrokeGroup(anchor = Anchor.Explicit(bg.elementRefs), strokes = groupStrokes)
    }
    val unanchored = strokes.filterIndexed { idx, _ -> idx !in usedIndices }
    return groups to unanchored
}

private fun elementsToJson(refs: List<ElementRef>): JSONArray {
    val arr = JSONArray()
    for (ref in refs) {
        arr.put(JSONObject().apply {
            putOpt("section_id", ref.sectionId)
            put("tag", ref.tag)
            put("text", ref.text)
        })
    }
    return arr
}

internal fun bindGroupsToJson(groups: List<BindGroup>): String {
    val arr = JSONArray()
    for (g in groups) {
        val obj = JSONObject()
        obj.put("id", g.id)
        obj.put("color", g.color)
        obj.put("strokeIndices", JSONArray(g.strokeIndices.toList()))
        obj.put("elementIndices", JSONArray(g.elementIndices))
        val refs = JSONArray()
        for (r in g.elementRefs) {
            refs.put(JSONObject().apply {
                put("section_id", r.sectionId)
                put("tag", r.tag)
                put("text", r.text)
            })
        }
        obj.put("elementRefs", refs)
        obj.put("markerDocX", g.markerDocX.toDouble())
        obj.put("markerDocY", g.markerDocY.toDouble())
        obj.put("strokeDocCenters", pairsToJson(g.strokeDocCenters))
        obj.put("elementDocCenters", pairsToJson(g.elementDocCenters))
        arr.put(obj)
    }
    return arr.toString()
}

internal fun bindGroupsFromJson(json: String): List<BindGroup> {
    val arr = JSONArray(json)
    return (0 until arr.length()).map { i ->
        val obj = arr.getJSONObject(i)
        val strokeIndices = obj.getJSONArray("strokeIndices").let { a ->
            (0 until a.length()).map { a.getInt(it) }.toSet()
        }
        val elementIndices = obj.getJSONArray("elementIndices").let { a ->
            (0 until a.length()).map { a.getInt(it) }
        }
        val refs = obj.getJSONArray("elementRefs").let { a ->
            (0 until a.length()).map { j ->
                val ro = a.getJSONObject(j)
                ElementRef(ro.optString("section_id", null), ro.getString("tag"), ro.optString("text", ""))
            }
        }
        BindGroup(
            id = obj.getInt("id"),
            color = obj.getInt("color"),
            strokeIndices = strokeIndices,
            elementIndices = elementIndices,
            elementRefs = refs,
            markerDocX = obj.getDouble("markerDocX").toFloat(),
            markerDocY = obj.getDouble("markerDocY").toFloat(),
            strokeDocCenters = pairsFromJson(obj.optJSONArray("strokeDocCenters") ?: JSONArray()),
            elementDocCenters = pairsFromJson(obj.optJSONArray("elementDocCenters") ?: JSONArray()),
        )
    }
}

private fun pairsToJson(pairs: List<Pair<Float, Float>>): JSONArray {
    val arr = JSONArray()
    for ((x, y) in pairs) arr.put(JSONArray().apply { put(x.toDouble()); put(y.toDouble()) })
    return arr
}

private fun pairsFromJson(arr: JSONArray): List<Pair<Float, Float>> =
    (0 until arr.length()).map { i ->
        val p = arr.getJSONArray(i)
        p.getDouble(0).toFloat() to p.getDouble(1).toFloat()
    }

internal fun pointInPolygon(px: Float, py: Float, polygon: List<Pair<Float, Float>>): Boolean {
    var inside = false
    var j = polygon.size - 1
    for (i in polygon.indices) {
        val (ix, iy) = polygon[i]
        val (jx, jy) = polygon[j]
        if ((iy > py) != (jy > py) && px < (jx - ix) * (py - iy) / (jy - iy) + ix) {
            inside = !inside
        }
        j = i
    }
    return inside
}

private fun strokesToPointArrays(strokes: List<Stroke>): JSONArray {
    val arr = JSONArray()
    for (stroke in strokes) {
        val pts = JSONArray()
        for ((x, y) in stroke.points) {
            pts.put(JSONArray().apply { put(x.toDouble()); put(y.toDouble()) })
        }
        arr.put(pts)
    }
    return arr
}
