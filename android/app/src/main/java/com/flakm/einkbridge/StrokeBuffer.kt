package com.flakm.einkbridge

import org.json.JSONArray
import org.json.JSONObject
import kotlin.math.max
import kotlin.math.min
import kotlin.math.sqrt

internal data class Stroke(val points: List<Pair<Float, Float>>, val width: Float)

internal data class ElementEntry(
    val i: Int,
    val tag: String,
    val id: String?,
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
    private var currentTransform = ViewTransform()

    val strokes: List<Stroke> get() = _strokes.toList()
    val isEmpty: Boolean get() = _strokes.isEmpty() && currentPoints.size < 2
    val size: Int get() = _strokes.size

    /** Returns committed strokes plus the in-progress stroke (if any). */
    fun allStrokes(): List<Stroke> {
        if (currentPoints.size < 2) return _strokes.toList()
        return _strokes + Stroke(currentPoints.toList(), currentWidth)
    }

    fun begin(x: Float, y: Float, width: Float = 3f, transform: ViewTransform = ViewTransform()) {
        currentTransform = transform
        val docW = width / transform.scale
        currentWidth = docW
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
            _strokes.add(Stroke(currentPoints.toList(), currentWidth))
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
            val pts = obj.getJSONArray("pts")
            val points = mutableListOf<Pair<Float, Float>>()
            for (j in 0 until pts.length()) {
                val p = pts.getJSONArray(j)
                points.add(p.getDouble(0).toFloat() to p.getDouble(1).toFloat())
            }
            if (points.size > 1) _strokes.add(Stroke(points, w))
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
    ElementRef(sectionId = el.id, tag = el.tag, text = el.text)

internal fun groupStrokesWithProximity(
    strokes: List<Stroke>,
    elements: List<ElementEntry>,
    explicitBindings: Map<Int, List<ElementEntry>> = emptyMap(),
    threshold: Float = 60f,
): Pair<List<StrokeGroup>, List<Stroke>> {
    val grouped = mutableMapOf<Int, MutableList<Stroke>>()
    val unanchored = mutableListOf<Stroke>()

    for ((idx, stroke) in strokes.withIndex()) {
        val explicitEls = explicitBindings[idx]
        if (explicitEls != null) {
            for (el in explicitEls) {
                grouped.getOrPut(el.i) { mutableListOf() }.add(stroke)
            }
            continue
        }
        if (elements.isEmpty()) {
            unanchored.add(stroke)
            continue
        }
        val (cx, cy) = strokeCentroid(stroke)
        val nearest = elements.minByOrNull { distToElement(cx, cy, it) }!!
        val dist = distToElement(cx, cy, nearest)
        if (dist <= threshold) {
            grouped.getOrPut(nearest.i) { mutableListOf() }.add(stroke)
        } else {
            unanchored.add(stroke)
        }
    }

    val explicitElementIndices = explicitBindings.values.flatten().map { it.i }.toSet()
    val groups = grouped.map { (elIdx, strks) ->
        val el = elements.first { it.i == elIdx }
        val anchor = if (elIdx in explicitElementIndices)
            Anchor.Explicit(listOf(elementToRef(el)))
        else
            Anchor.Proximity(listOf(elementToRef(el)))
        StrokeGroup(anchor = anchor, strokes = strks)
    }
    return groups to unanchored
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

private fun elementsToJson(refs: List<ElementRef>): JSONArray {
    val arr = JSONArray()
    for (ref in refs) {
        arr.put(JSONObject().apply {
            put("section_id", ref.sectionId)
            put("tag", ref.tag)
            put("text", ref.text)
        })
    }
    return arr
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
