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

    private val paint = Paint().apply {
        color = Color.BLACK
        style = Paint.Style.STROKE
        isAntiAlias = true
        strokeCap = Paint.Cap.ROUND
        strokeJoin = Paint.Join.ROUND
    }

    fun update(strokes: List<Stroke>, transform: ViewTransform = ViewTransform()) {
        this.strokes = strokes
        this.transform = transform
        invalidate()
    }

    fun updateTransform(transform: ViewTransform) {
        this.transform = transform
        invalidate()
    }

    override fun onDraw(canvas: Canvas) {
        val t = transform
        for (stroke in strokes) {
            if (stroke.points.size < 2) continue
            paint.strokeWidth = t.docToScreenWidth(stroke.width)
            // Use drawLine instead of Path to guarantee no implicit path closing.
            var prevX = t.docToScreenX(stroke.points[0].first)
            var prevY = t.docToScreenY(stroke.points[0].second)
            for (i in 1 until stroke.points.size) {
                val curX = t.docToScreenX(stroke.points[i].first)
                val curY = t.docToScreenY(stroke.points[i].second)
                canvas.drawLine(prevX, prevY, curX, curY, paint)
                prevX = curX
                prevY = curY
            }
        }
    }

}
