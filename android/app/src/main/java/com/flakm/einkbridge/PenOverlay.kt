package com.flakm.einkbridge

import android.graphics.*
import android.util.Log
import android.view.View
import android.webkit.WebView
import com.onyx.android.sdk.data.note.TouchPoint
import com.onyx.android.sdk.pen.RawInputCallback
import com.onyx.android.sdk.pen.TouchHelper
import com.onyx.android.sdk.pen.data.TouchPointList
import java.io.ByteArrayOutputStream

class PenOverlay(private val webView: WebView, private val excludeView: View? = null) {
    private val tag = "PenOverlay"
    private var touchHelper: TouchHelper? = null
    private val strokes = mutableListOf<List<PointF>>()
    private var currentStroke = mutableListOf<PointF>()
    private var initialized = false
    var isDrawingEnabled = false
        private set

    private val callback = object : RawInputCallback() {
        override fun onBeginRawDrawing(b: Boolean, touchPoint: TouchPoint) {
            Log.d(tag, "onBeginRawDrawing ${touchPoint.x}, ${touchPoint.y}")
            currentStroke = mutableListOf()
            currentStroke.add(PointF(touchPoint.x, touchPoint.y))
        }

        override fun onEndRawDrawing(b: Boolean, touchPoint: TouchPoint) {
            Log.d(tag, "onEndRawDrawing, stroke size: ${currentStroke.size}")
            currentStroke.add(PointF(touchPoint.x, touchPoint.y))
            if (currentStroke.size > 1) {
                strokes.add(currentStroke.toList())
            }
            currentStroke = mutableListOf()
        }

        override fun onRawDrawingTouchPointMoveReceived(touchPoint: TouchPoint) {
            currentStroke.add(PointF(touchPoint.x, touchPoint.y))
        }

        override fun onRawDrawingTouchPointListReceived(touchPointList: TouchPointList) {
            Log.d(tag, "onRawDrawingTouchPointListReceived size: ${touchPointList.size()}")
            for (i in 0 until touchPointList.size()) {
                val tp = touchPointList.get(i)
                currentStroke.add(PointF(tp.x, tp.y))
            }
        }

        override fun onBeginRawErasing(b: Boolean, touchPoint: TouchPoint) {}
        override fun onEndRawErasing(b: Boolean, touchPoint: TouchPoint) {}
        override fun onRawErasingTouchPointMoveReceived(touchPoint: TouchPoint) {}
        override fun onRawErasingTouchPointListReceived(touchPointList: TouchPointList) {}
    }

    private val layoutListener = object : View.OnLayoutChangeListener {
        override fun onLayoutChange(
            v: View, left: Int, top: Int, right: Int, bottom: Int,
            oldLeft: Int, oldTop: Int, oldRight: Int, oldBottom: Int
        ) {
            val w = right - left
            val h = bottom - top
            if (w > 0 && h > 0 && !initialized) {
                Log.d(tag, "View laid out: ${w}x${h}, creating TouchHelper")
                initTouchHelper()
            }
        }
    }

    fun init() {
        webView.addOnLayoutChangeListener(layoutListener)
        // If already laid out, init immediately
        if (webView.width > 0 && webView.height > 0) {
            webView.post { initTouchHelper() }
        }
    }

    private fun initTouchHelper() {
        if (initialized) return
        touchHelper?.closeRawDrawing()

        val helper = TouchHelper.create(webView, callback)
        touchHelper = helper

        val limit = Rect()
        webView.getLocalVisibleRect(limit)
        Log.d(tag, "initTouchHelper limit=$limit")

        if (limit.width() <= 0 || limit.height() <= 0) {
            Log.e(tag, "Invalid limit rect, deferring")
            return
        }

        val excludeRects = mutableListOf<Rect>()
        if (excludeView != null) {
            val parentLoc = IntArray(2)
            val childLoc = IntArray(2)
            webView.getLocationOnScreen(parentLoc)
            excludeView.getLocationOnScreen(childLoc)
            val r = Rect()
            excludeView.getLocalVisibleRect(r)
            r.offset(childLoc[0] - parentLoc[0], childLoc[1] - parentLoc[1])
            excludeRects.add(r)
            Log.d(tag, "Excluding toolbar rect: $r")
        }

        helper.setStrokeWidth(3.0f)
            .setLimitRect(limit, excludeRects)
            .openRawDrawing()
        helper.setRawDrawingRenderEnabled(true)
        helper.setStrokeStyle(TouchHelper.STROKE_STYLE_PENCIL)
        helper.setRawDrawingEnabled(false) // start in read mode
        initialized = true
        Log.d(tag, "TouchHelper initialized, raw drawing ready")
    }

    private var currentWidth = 3.0f

    fun enableDrawing() {
        if (!initialized) {
            Log.w(tag, "enableDrawing called before init, trying init")
            initTouchHelper()
        }
        touchHelper?.setRawDrawingEnabled(true)
        isDrawingEnabled = true
        Log.d(tag, "Drawing enabled")
    }

    fun disableDrawing() {
        touchHelper?.setRawDrawingEnabled(false)
        isDrawingEnabled = false
        Log.d(tag, "Drawing disabled")
    }

    fun setStrokeWidth(width: Float) {
        currentWidth = width
        touchHelper?.setStrokeWidth(width)
        Log.d(tag, "Stroke width: $width")
    }

    fun setStylePencil() {
        touchHelper?.setStrokeStyle(TouchHelper.STROKE_STYLE_PENCIL)
        Log.d(tag, "Style: pencil")
    }

    fun setStyleBrush() {
        touchHelper?.setStrokeStyle(TouchHelper.STROKE_STYLE_FOUNTAIN)
        Log.d(tag, "Style: brush")
    }

    fun clearStrokes() {
        strokes.clear()
        currentStroke.clear()
        webView.reload()
    }

    fun hasStrokes(): Boolean = strokes.isNotEmpty()

    fun exportToPng(): ByteArray? {
        if (strokes.isEmpty()) return null
        val w = webView.width
        val h = webView.height
        if (w <= 0 || h <= 0) return null

        val bitmap = Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bitmap)
        canvas.drawColor(Color.TRANSPARENT)

        val paint = Paint().apply {
            color = Color.BLACK
            strokeWidth = currentWidth
            style = Paint.Style.STROKE
            isAntiAlias = true
            strokeCap = Paint.Cap.ROUND
            strokeJoin = Paint.Join.ROUND
        }

        for (stroke in strokes) {
            if (stroke.size < 2) continue
            val path = Path()
            path.moveTo(stroke[0].x, stroke[0].y)
            for (i in 1 until stroke.size) {
                path.lineTo(stroke[i].x, stroke[i].y)
            }
            canvas.drawPath(path, paint)
        }

        val out = ByteArrayOutputStream()
        bitmap.compress(Bitmap.CompressFormat.PNG, 100, out)
        bitmap.recycle()
        Log.d(tag, "Exported ${strokes.size} strokes to PNG (${out.size()} bytes)")
        return out.toByteArray()
    }

    fun destroy() {
        webView.removeOnLayoutChangeListener(layoutListener)
        touchHelper?.setRawDrawingEnabled(false)
        touchHelper?.closeRawDrawing()
        touchHelper = null
        initialized = false
    }
}
