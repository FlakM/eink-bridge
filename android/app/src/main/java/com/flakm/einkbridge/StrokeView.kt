package com.flakm.einkbridge

import android.content.Context
import android.graphics.*
import android.graphics.Typeface
import android.util.AttributeSet
import android.view.View

internal sealed class LabelId {
    data class Group(val groupId: Int) : LabelId()
    data class Cluster(val clusterIdx: Int) : LabelId()
}

internal class StrokeView @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
) : View(context, attrs) {

    internal var strokes: List<Stroke> = emptyList()
        private set
    private var transform = ViewTransform()
    private var bindGroups: List<BindGroup> = emptyList()
    private var ocrResults: List<OcrResult> = emptyList()
    private var bindPathPoints: List<PointF>? = null
    private var expandedGroupIds = mutableSetOf<Int>()
    private var annotationMode = false
    var bindDrawingActive = false
    private var groupLabelOffsets: Map<Int, Pair<Float, Float>> = emptyMap()
    private var clusterLabelOffsets: Map<Int, Pair<Float, Float>> = emptyMap()
    private var selectedLabel: LabelId? = null

    private val strokePaint = Paint().apply {
        style = Paint.Style.STROKE
        isAntiAlias = true
        strokeCap = Paint.Cap.ROUND
        strokeJoin = Paint.Join.ROUND
    }

    private val dashPaint = Paint().apply {
        style = Paint.Style.STROKE
        strokeWidth = 6f
        isAntiAlias = true
        pathEffect = DashPathEffect(floatArrayOf(18f, 10f), 0f)
        strokeCap = Paint.Cap.ROUND
    }

    private val bindPathPaint = Paint().apply {
        color = 0xCCFF9800.toInt()
        style = Paint.Style.STROKE
        strokeWidth = 5f
        isAntiAlias = true
        strokeCap = Paint.Cap.ROUND
        strokeJoin = Paint.Join.ROUND
    }

    private val bindFillPaint = Paint().apply {
        color = 0x22FF9800.toInt()
        style = Paint.Style.FILL
        isAntiAlias = true
    }

    private val markerPaint = Paint().apply {
        isAntiAlias = true
        style = Paint.Style.FILL
    }

    private val ocrBadgePaint = Paint().apply {
        isAntiAlias = true
        style = Paint.Style.FILL
        color = 0xFF444444.toInt()
    }

    private val ocrTextPaint = Paint().apply {
        isAntiAlias = true
        color = Color.WHITE
        textSize = 26f
        textAlign = Paint.Align.CENTER
        typeface = Typeface.DEFAULT_BOLD
    }

    private val ocrBoxBgPaint = Paint().apply {
        isAntiAlias = true
        style = Paint.Style.FILL
        color = 0xF0FFFFFF.toInt()
    }

    private val ocrBoxBorderPaint = Paint().apply {
        isAntiAlias = true
        style = Paint.Style.STROKE
        strokeWidth = 3f
        color = 0xFF555555.toInt()
    }

    private val ocrLabelPaint = Paint().apply {
        isAntiAlias = true
        color = Color.BLACK
        textSize = 34f
        textAlign = Paint.Align.CENTER
    }

    private val linkLinePaint = Paint().apply {
        style = Paint.Style.STROKE
        strokeWidth = 3f
        isAntiAlias = true
        strokeCap = Paint.Cap.ROUND
    }

    private val selectionPaint = Paint().apply {
        style = Paint.Style.STROKE
        strokeWidth = 4f
        isAntiAlias = true
        color = Color.BLACK
        pathEffect = DashPathEffect(floatArrayOf(10f, 6f), 0f)
    }

    private var cachedLabelRects: List<Pair<LabelId, RectF>> = emptyList()

    fun update(
        strokes: List<Stroke>,
        transform: ViewTransform = ViewTransform(),
        bindGroups: List<BindGroup> = emptyList(),
        bindPath: List<PointF>? = null,
        ocrResults: List<OcrResult> = emptyList(),
        annotationMode: Boolean = false,
        groupLabelOffsets: Map<Int, Pair<Float, Float>> = emptyMap(),
        clusterLabelOffsets: Map<Int, Pair<Float, Float>> = emptyMap(),
        selectedLabel: LabelId? = null,
    ) {
        this.strokes = strokes
        this.transform = transform
        this.bindGroups = bindGroups
        this.bindPathPoints = bindPath
        this.ocrResults = ocrResults
        this.annotationMode = annotationMode
        this.groupLabelOffsets = groupLabelOffsets
        this.clusterLabelOffsets = clusterLabelOffsets
        this.selectedLabel = selectedLabel
        invalidate()
    }

    fun updateTransform(transform: ViewTransform) {
        this.transform = transform
        invalidate()
    }

    fun setBindPath(path: List<PointF>?) {
        bindPathPoints = path
        invalidate()
    }

    fun showGroup(groupId: Int, durationMs: Long = SHOW_DURATION_MS) {
        expandedGroupIds.add(groupId)
        invalidate()
        handler?.postDelayed({
            expandedGroupIds.remove(groupId)
            invalidate()
        }, durationMs)
    }

    fun isGroupExpanded(groupId: Int): Boolean = groupId in expandedGroupIds

    fun flashGroup(groupId: Int) = showGroup(groupId)

    fun setLasso(rect: RectF?) { /* kept for API compat, no-op */ }

    fun hitTestLabel(screenX: Float, screenY: Float): LabelId? =
        cachedLabelRects.firstOrNull { (_, rect) -> rect.contains(screenX, screenY) }?.first

    override fun onDraw(canvas: Canvas) {
        val t = transform
        val expanded = expandedGroupIds

        val highlightedStrokes = mutableMapOf<Int, Int>()
        for (group in bindGroups) {
            if (group.id in expanded) {
                for (si in group.strokeIndices) {
                    highlightedStrokes[si] = group.color
                }
            }
        }

        val widthScale = if (bindDrawingActive) 2.5f else 1f

        for ((idx, stroke) in strokes.withIndex()) {
            if (stroke.points.size < 2) continue
            val hlColor = highlightedStrokes[idx]
            strokePaint.color = hlColor ?: stroke.color
            val baseWidth = t.docToScreenWidth(stroke.width) * widthScale
            strokePaint.strokeWidth = if (hlColor != null) baseWidth * 1.5f else baseWidth
            var prevX = t.docToScreenX(stroke.points[0].first)
            var prevY = t.docToScreenY(stroke.points[0].second)
            for (i in 1 until stroke.points.size) {
                val curX = t.docToScreenX(stroke.points[i].first)
                val curY = t.docToScreenY(stroke.points[i].second)
                canvas.drawLine(prevX, prevY, curX, curY, strokePaint)
                prevX = curX
                prevY = curY
            }
        }

        bindPathPoints?.let { pts ->
            if (pts.size >= 2) {
                val path = Path().apply {
                    moveTo(pts[0].x, pts[0].y)
                    for (i in 1 until pts.size) lineTo(pts[i].x, pts[i].y)
                }
                val fillPath = Path(path).apply { close() }
                canvas.drawPath(fillPath, bindFillPaint)
                canvas.drawPath(path, bindPathPaint)
            }
        }

        if (!annotationMode) return

        // --- Pass 1: compute natural positions for all labels ---
        val labels = mutableListOf<AnnotLabel>()

        for (group in bindGroups) {
            val offset = groupLabelOffsets[group.id] ?: (0f to 0f)
            val sx = t.docToScreenX(group.markerDocX) + offset.first * t.scale
            val pillText = group.recognizedText ?: ""
            val pillPad = 16f
            val w = maxOf((if (pillText.isNotEmpty()) ocrLabelPaint.measureText(pillText) else 0f) + pillPad * 2, MARKER_RADIUS_PX * 2)
            val h = if (pillText.isNotEmpty()) ocrLabelPaint.textSize + pillPad * 2 else MARKER_RADIUS_PX * 2
            var minDocY = Float.MAX_VALUE
            for (idx in group.strokeIndices) {
                val stroke = strokes.getOrNull(idx) ?: continue
                for ((_, y) in stroke.points) if (y < minDocY) minDocY = y
            }
            val anchorY = (if (minDocY < Float.MAX_VALUE) t.docToScreenY(minDocY) else t.docToScreenY(group.markerDocY)) +
                offset.second * t.scale
            labels.add(AnnotLabel.Group(sx, anchorY - LABEL_GAP - h, w, h, group))
        }

        for ((idx, result) in ocrResults.withIndex()) {
            val offset = clusterLabelOffsets[idx] ?: (0f to 0f)
            val sx = t.docToScreenX(result.docX) + offset.first * t.scale
            val pad = 10f
            val w = ocrLabelPaint.measureText(result.text) + pad * 2
            val h = ocrLabelPaint.textSize + pad * 2
            val topY = t.docToScreenY(result.minDocY) - LABEL_GAP - h + offset.second * t.scale
            labels.add(AnnotLabel.Cluster(sx, topY, w, h, result.text, idx))
        }

        // --- Pass 2: push overlapping labels upward ---
        val placed = mutableListOf<RectF>()
        for (label in labels.sortedByDescending { it.topY }) {
            var topY = label.topY
            var retry = true
            while (retry) {
                retry = false
                for (p in placed) {
                    val lx1 = label.sx - label.w / 2; val lx2 = label.sx + label.w / 2
                    if (lx2 <= p.left || lx1 >= p.right) continue
                    if (topY + label.h <= p.top || topY >= p.bottom) continue
                    topY = p.top - label.h - LABEL_GAP
                    retry = true; break
                }
            }
            label.topY = topY
            placed.add(RectF(label.sx - label.w / 2, topY, label.sx + label.w / 2, topY + label.h))
        }

        // Cache rects for hit-testing after positions are finalised
        cachedLabelRects = labels.map { label ->
            val rect = RectF(label.sx - label.w / 2, label.topY, label.sx + label.w / 2, label.topY + label.h)
            val id: LabelId = when (label) {
                is AnnotLabel.Group -> LabelId.Group(label.group.id)
                is AnnotLabel.Cluster -> LabelId.Cluster(label.clusterIdx)
            }
            id to rect
        }

        // --- Pass 3: draw lines first, then labels on top ---
        for (label in labels) {
            if (label !is AnnotLabel.Group) continue
            val centerY = label.topY + label.h / 2
            val dimColor = (label.group.color and 0x00FFFFFF) or 0x66000000
            dashPaint.color = dimColor; dashPaint.strokeWidth = 2f
            for ((cx, cy) in label.group.strokeDocCenters)
                canvas.drawLine(label.sx, centerY, t.docToScreenX(cx), t.docToScreenY(cy), dashPaint)
            linkLinePaint.color = dimColor; linkLinePaint.strokeWidth = 2f
            for ((cx, cy) in label.group.elementDocCenters)
                canvas.drawLine(label.sx, centerY, t.docToScreenX(cx), t.docToScreenY(cy), linkLinePaint)
        }

        for (label in labels) {
            val rect = RectF(label.sx - label.w / 2, label.topY, label.sx + label.w / 2, label.topY + label.h)
            val centerY = label.topY + label.h / 2
            when (label) {
                is AnnotLabel.Group -> {
                    val cornerR = label.h / 2
                    markerPaint.color = Color.WHITE; markerPaint.style = Paint.Style.FILL
                    canvas.drawRoundRect(rect, cornerR, cornerR, markerPaint)
                    markerPaint.color = label.group.color; markerPaint.style = Paint.Style.STROKE; markerPaint.strokeWidth = 5f
                    canvas.drawRoundRect(rect, cornerR, cornerR, markerPaint)
                    markerPaint.style = Paint.Style.FILL
                    val text = label.group.recognizedText
                    if (!text.isNullOrEmpty())
                        canvas.drawText(text, label.sx, centerY + ocrLabelPaint.textSize * 0.35f, ocrLabelPaint)
                }
                is AnnotLabel.Cluster -> {
                    canvas.drawRoundRect(rect, 6f, 6f, ocrBoxBgPaint)
                    canvas.drawRoundRect(rect, 6f, 6f, ocrBoxBorderPaint)
                    canvas.drawText(label.text, label.sx, label.topY + label.h - 10f, ocrLabelPaint)
                }
            }
        }

        // Selection highlight
        val sel = selectedLabel
        if (sel != null) {
            for ((id, rect) in cachedLabelRects) {
                if (id == sel) {
                    val ex = RectF(rect.left - 8f, rect.top - 8f, rect.right + 8f, rect.bottom + 8f)
                    canvas.drawRoundRect(ex, 14f, 14f, selectionPaint)
                }
            }
        }
    }

    private sealed class AnnotLabel {
        abstract val sx: Float
        abstract var topY: Float
        abstract val w: Float
        abstract val h: Float

        data class Group(override val sx: Float, override var topY: Float, override val w: Float, override val h: Float, val group: BindGroup) : AnnotLabel()
        data class Cluster(override val sx: Float, override var topY: Float, override val w: Float, override val h: Float, val text: String, val clusterIdx: Int) : AnnotLabel()
    }

    companion object {
        const val UNBOUND_MARKER_COLOR = 0xFF888888.toInt()
        const val MARKER_RADIUS_PX = 28f
        const val LABEL_GAP = 12f
        const val BADGE_RADIUS_PX = 18f
        const val SHOW_DURATION_MS = 3000L
        const val MIN_OCR_DIAGONAL_PX = 80f

        fun groupScreenDiagonal(group: BindGroup, strokes: List<Stroke>, t: ViewTransform): Float {
            var minX = Float.MAX_VALUE; var maxX = Float.MIN_VALUE
            var minY = Float.MAX_VALUE; var maxY = Float.MIN_VALUE
            for (idx in group.strokeIndices) {
                val stroke = strokes.getOrNull(idx) ?: continue
                for ((x, y) in stroke.points) {
                    if (x < minX) minX = x; if (x > maxX) maxX = x
                    if (y < minY) minY = y; if (y > maxY) maxY = y
                }
            }
            if (minX > maxX) return 0f
            val sw = t.docToScreenWidth(maxX - minX)
            val sh = t.docToScreenWidth(maxY - minY)
            return kotlin.math.sqrt((sw * sw + sh * sh).toDouble()).toFloat()
        }
    }
}
