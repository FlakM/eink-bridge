package com.flakm.einkbridge

import android.content.Context
import android.graphics.*
import android.util.AttributeSet
import android.view.View

internal class StrokeView @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
) : View(context, attrs) {

    internal var strokes: List<Stroke> = emptyList()
        private set
    private var transform = ViewTransform()
    private var bindGroups: List<BindGroup> = emptyList()
    private var bindPathPoints: List<PointF>? = null
    private var expandedGroupIds = mutableSetOf<Int>()
    var bindDrawingActive = false

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

    fun update(
        strokes: List<Stroke>,
        transform: ViewTransform = ViewTransform(),
        bindGroups: List<BindGroup> = emptyList(),
        bindPath: List<PointF>? = null,
    ) {
        this.strokes = strokes
        this.transform = transform
        this.bindGroups = bindGroups
        this.bindPathPoints = bindPath
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

        for (group in bindGroups) {
            val isExpanded = group.id in expanded
            val sx = t.docToScreenX(group.markerDocX)
            val sy = t.docToScreenY(group.markerDocY)

            if (isExpanded) {
                val semiColor = (group.color and 0x00FFFFFF) or (0xCC shl 24)
                dashPaint.color = semiColor
                for ((cx, cy) in group.strokeDocCenters) {
                    canvas.drawLine(sx, sy, t.docToScreenX(cx), t.docToScreenY(cy), dashPaint)
                }
                for ((cx, cy) in group.elementDocCenters) {
                    canvas.drawLine(sx, sy, t.docToScreenX(cx), t.docToScreenY(cy), dashPaint)
                }
            }

            val radius = if (isExpanded) MARKER_RADIUS_PX * 1.3f else MARKER_RADIUS_PX
            markerPaint.color = group.color
            markerPaint.style = Paint.Style.FILL
            canvas.drawCircle(sx, sy, radius, markerPaint)
            markerPaint.color = Color.WHITE
            markerPaint.style = Paint.Style.STROKE
            markerPaint.strokeWidth = 3f
            canvas.drawCircle(sx, sy, radius, markerPaint)
            markerPaint.style = Paint.Style.FILL
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
    }

    companion object {
        const val MARKER_RADIUS_PX = 28f
        const val SHOW_DURATION_MS = 3000L
    }
}
