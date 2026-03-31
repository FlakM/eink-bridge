package com.flakm.einkbridge

import android.annotation.SuppressLint
import android.graphics.*
import android.graphics.Rect
import android.view.MotionEvent
import android.view.View
import android.webkit.WebView
import com.onyx.android.sdk.api.device.epd.EpdController
import com.onyx.android.sdk.data.note.TouchPoint
import com.onyx.android.sdk.pen.RawInputCallback
import com.onyx.android.sdk.pen.TouchHelper
import com.onyx.android.sdk.pen.data.TouchPointList
import org.json.JSONArray
import org.json.JSONObject
import java.io.ByteArrayOutputStream

/**
 * Pure function — no Android framework dependency beyond integer constants.
 *
 * Returns:
 *   true  → enable raw drawing (finger gesture ended)
 *   false → disable raw drawing (finger gesture began)
 *   null  → no action needed (stylus event or unhandled action)
 */
internal fun rawDrawingAction(
    pointerCount: Int,
    getToolType: (Int) -> Int,
    actionMasked: Int,
): Boolean? {
    val toolTypeStylus = 2  // MotionEvent.TOOL_TYPE_STYLUS
    val toolTypeEraser = 4  // MotionEvent.TOOL_TYPE_ERASER
    val actionDown = 0      // MotionEvent.ACTION_DOWN
    val actionUp = 1        // MotionEvent.ACTION_UP
    val actionCancel = 3    // MotionEvent.ACTION_CANCEL

    val hasStylus = (0 until pointerCount).any {
        val t = getToolType(it); t == toolTypeStylus || t == toolTypeEraser
    }
    if (hasStylus) return null
    return when (actionMasked) {
        actionDown -> false
        actionUp, actionCancel -> true
        else -> null
    }
}

/** Production controller -- wraps the Onyx TouchHelper SDK. */
internal class OnyxPenController(
    private val buf: StrokeBuffer,
    private val getTransform: () -> ViewTransform,
    private val onEraseApplied: () -> Unit = {},
    internal var onStrokeProgress: (() -> Unit)? = null,
) : PenInputController {
    private var touchHelper: TouchHelper? = null
    private var penViewRef: java.lang.ref.WeakReference<View>? = null
    private var pendingWidth: Float? = null
    private var eraseMode = false
    private var eraserPath = mutableListOf<Pair<Float, Float>>()
    private var currentWidth = 3f
    private var moveCount = 0

    private val callback = object : RawInputCallback() {
        override fun onBeginRawDrawing(b: Boolean, tp: TouchPoint) {
            moveCount = 0
            if (eraseMode) {
                eraserPath = mutableListOf(tp.x to tp.y)
            } else {
                pendingWidth?.let {
                    touchHelper?.setStrokeWidth(it)
                    currentWidth = it
                    pendingWidth = null
                }
                buf.begin(tp.x, tp.y, currentWidth, getTransform())
                onStrokeProgress?.invoke()
            }
        }
        override fun onEndRawDrawing(b: Boolean, tp: TouchPoint) {
            if (eraseMode) {
                eraserPath.add(tp.x to tp.y)
                buf.erase(eraserPath, ERASER_RADIUS, getTransform())
                eraserPath.clear()
                onEraseApplied()
            } else {
                buf.commit()
                onStrokeProgress?.invoke()
            }
            // Briefly disable raw drawing so pending finger taps on the
            // toolbar aren't blocked, then re-enable for the next pen stroke.
            val helper = touchHelper ?: return
            helper.setRawDrawingEnabled(false)
            penViewRef?.get()?.postDelayed({ helper.setRawDrawingEnabled(true) }, 50)
        }
        override fun onRawDrawingTouchPointMoveReceived(tp: TouchPoint) {
            moveCount++
            if (eraseMode) eraserPath.add(tp.x to tp.y) else buf.addPoint(tp.x, tp.y)
            if (moveCount % 3 == 0) onStrokeProgress?.invoke()
        }
        override fun onRawDrawingTouchPointListReceived(list: TouchPointList) {
            // Ignored: points already delivered individually via move callback
        }
        override fun onBeginRawErasing(b: Boolean, tp: TouchPoint) {
            eraserPath = mutableListOf(tp.x to tp.y)
        }
        override fun onRawErasingTouchPointMoveReceived(tp: TouchPoint) {
            eraserPath.add(tp.x to tp.y)
        }
        override fun onRawErasingTouchPointListReceived(list: TouchPointList) {}
        override fun onEndRawErasing(b: Boolean, tp: TouchPoint) {
            eraserPath.add(tp.x to tp.y)
            buf.erase(eraserPath, ERASER_RADIUS, getTransform())
            eraserPath.clear()
            onEraseApplied()
            val helper = touchHelper ?: return
            helper.setRawDrawingEnabled(false)
            penViewRef?.get()?.postDelayed({ helper.setRawDrawingEnabled(true) }, 50)
        }
    }

    override fun open(view: View, limitRect: Rect, excludeRects: List<Rect>) {
        val helper = TouchHelper.create(view, callback)
        touchHelper = helper
        penViewRef = java.lang.ref.WeakReference(view)
        helper.setStrokeWidth(3.0f)
            .setLimitRect(limitRect, excludeRects)
            .openRawDrawing()
        helper.setRawDrawingRenderEnabled(false)
        helper.setStrokeStyle(TouchHelper.STROKE_STYLE_PENCIL)
        helper.setRawDrawingEnabled(true)
    }

    override fun updateLimitRect(limitRect: Rect, excludeRects: List<Rect>) {
        touchHelper?.setLimitRect(limitRect, excludeRects)
    }

    override fun setEnabled(enabled: Boolean) { touchHelper?.setRawDrawingEnabled(enabled) }
    override fun setStrokeWidth(width: Float) { pendingWidth = width }
    override fun setStylePencil() {
        eraseMode = false
        touchHelper?.setStrokeStyle(TouchHelper.STROKE_STYLE_PENCIL)
    }
    override fun setStyleBrush() {
        eraseMode = false
        touchHelper?.setStrokeStyle(TouchHelper.STROKE_STYLE_FOUNTAIN)
    }
    override fun setStyleEraser() { eraseMode = true }

    override fun resetRenderBuffer() {
        val helper = touchHelper ?: return
        helper.setRawDrawingEnabled(false)
        helper.closeRawDrawing()
        helper.openRawDrawing()
        helper.setRawDrawingRenderEnabled(false)
        helper.setRawDrawingEnabled(true)
    }

    override fun close() {
        touchHelper?.setRawDrawingEnabled(false)
        touchHelper?.closeRawDrawing()
        touchHelper = null
    }

    companion object {
        const val ERASER_RADIUS = 30f
    }
}

internal class PenOverlay(
    private val webView: WebView,
    private val excludeView: View? = null,
    private val strokeView: StrokeView? = null,
    internal val buf: StrokeBuffer = StrokeBuffer(),
    private val controllerOverride: PenInputController? = null,
    internal val limitRectOverride: Rect? = null,
    internal val transformOverride: (() -> ViewTransform)? = null,
) {
    private val controller: PenInputController by lazy {
        val c = controllerOverride ?: OnyxPenController(buf, ::currentTransform, onEraseApplied = {
            notifyStrokeView()
            controller.resetRenderBuffer()
        }, onStrokeProgress = { notifyStrokeViewLive() })
        if (c is OnyxPenController && c.onStrokeProgress == null) {
            c.onStrokeProgress = { notifyStrokeViewLive() }
        }
        c
    }
    private var initialized = false

    @Suppress("DEPRECATION")
    internal fun currentTransform(): ViewTransform {
        transformOverride?.let { return it() }
        val scale = try { webView.scale } catch (_: ClassCastException) { 1f }
        return ViewTransform(
            scrollX = webView.scrollX.toFloat(),
            scrollY = webView.scrollY.toFloat(),
            scale = scale,
        )
    }

    private val layoutListener = object : View.OnLayoutChangeListener {
        override fun onLayoutChange(
            v: View, left: Int, top: Int, right: Int, bottom: Int,
            oldLeft: Int, oldTop: Int, oldRight: Int, oldBottom: Int,
        ) {
            if (!initialized) maybeInit()
            else if (v === excludeView) refreshExcludeRects()
        }
    }

    @SuppressLint("ClickableViewAccessibility")
    private val touchRouter = View.OnTouchListener { _, event ->
        val action = rawDrawingAction(event.pointerCount, event::getToolType, event.actionMasked)
        if (action != null) {
            if (!action) {
                notifyStrokeView()
                controller.setEnabled(false)
            } else {
                controller.resetRenderBuffer()
                notifyStrokeView()
            }
            false
        } else {
            // Stylus/eraser event — consume it so the WebView doesn't scroll.
            val hasPen = (0 until event.pointerCount).any {
                val t = event.getToolType(it)
                t == MotionEvent.TOOL_TYPE_STYLUS || t == MotionEvent.TOOL_TYPE_ERASER
            }
            hasPen
        }
    }

    private val scrollListener = object : View.OnScrollChangeListener {
        override fun onScrollChange(v: View, scrollX: Int, scrollY: Int, oldScrollX: Int, oldScrollY: Int) {
            strokeView?.updateTransform(currentTransform())
        }
    }

    @SuppressLint("ClickableViewAccessibility")
    fun init() {
        webView.addOnLayoutChangeListener(layoutListener)
        excludeView?.addOnLayoutChangeListener(layoutListener)
        webView.setOnTouchListener(touchRouter)
        webView.setOnScrollChangeListener(scrollListener)
        // Poll scale changes during pinch-zoom via touch events
        setupScaleTracker()
        maybeInit()
    }

    /**
     * The WebView doesn't fire onScrollChange for scale changes (pinch-zoom).
     * We piggyback on ACTION_MOVE during finger gestures to update the transform.
     */
    private fun setupScaleTracker() {
        val origListener = touchRouter
        @SuppressLint("ClickableViewAccessibility")
        val wrappedListener = View.OnTouchListener { v, event ->
            val result = origListener.onTouch(v, event)
            if (event.actionMasked == MotionEvent.ACTION_MOVE && event.pointerCount >= 2) {
                strokeView?.updateTransform(currentTransform())
            }
            result
        }
        webView.setOnTouchListener(wrappedListener)
    }

    private fun maybeInit() {
        if (initialized) return
        val wvReady = limitRectOverride != null || (webView.width > 0 && webView.height > 0)
        val exReady = excludeView == null || excludeView.height > 0
        if (wvReady && exReady) {
            initController()
        } else if (wvReady) {
            webView.post { maybeInit() }
        }
    }

    private fun initController() {
        if (initialized) return
        val limit = visibleRect() ?: return
        controller.open(webView, limit, buildExcludeRects(limit))
        initialized = true
    }

    private fun refreshExcludeRects() {
        val limit = visibleRect() ?: return
        controller.updateLimitRect(limit, buildExcludeRects(limit))
    }

    private fun visibleRect(): Rect? {
        if (limitRectOverride != null) return Rect(limitRectOverride)
        val r = Rect()
        if (!webView.getLocalVisibleRect(r)) r.set(0, 0, webView.width, webView.height)
        if (r.width() <= 0 || r.height() <= 0) return null
        return r
    }

    private fun buildExcludeRects(limit: Rect): List<Rect> {
        val h = excludeView?.height ?: 0
        if (h <= 0) return emptyList()
        return listOf(Rect(limit.left, limit.bottom - h, limit.right, limit.bottom))
    }

    fun enableDrawing() {
        if (!initialized) initController()
        controller.setEnabled(true)
    }

    fun disableDrawing() { controller.setEnabled(false) }

    fun setStrokeWidth(width: Float) { controller.setStrokeWidth(width) }
    fun setStylePencil() { controller.setStylePencil() }
    fun setStyleBrush() { controller.setStyleBrush() }
    fun setStyleEraser() { controller.setStyleEraser() }

    fun undoLastStroke() {
        buf.undo()
        notifyStrokeView()
        controller.resetRenderBuffer()
    }

    fun clearStrokes() {
        buf.clear()
        notifyStrokeView()
        controller.resetRenderBuffer()
    }

    private fun notifyStrokeView() {
        strokeView?.update(buf.strokes, currentTransform())
    }

    private fun notifyStrokeViewLive() {
        strokeView?.update(buf.allStrokes(), currentTransform())
    }

    fun exportToPng(): ByteArray? {
        if (buf.isEmpty) return null
        val w = webView.width
        val h = webView.height
        if (w <= 0 || h <= 0) return null
        return renderStrokesToPng(w, h, buf.strokes, currentTransform())
    }

    fun exportStrokeJson(): String? {
        if (buf.isEmpty) return null
        val rect = visibleRect() ?: return null
        val w = rect.width()
        val h = rect.height()
        val t = currentTransform()
        val strokesArr = JSONArray()
        for (stroke in buf.strokes) {
            val strokeArr = JSONArray()
            for ((dx, dy) in stroke.points) {
                strokeArr.put(JSONArray().apply {
                    put(t.docToScreenX(dx).toDouble())
                    put(t.docToScreenY(dy).toDouble())
                })
            }
            strokesArr.put(strokeArr)
        }
        return JSONObject().apply {
            put("canvas_width", w)
            put("canvas_height", h)
            put("strokes", strokesArr)
        }.toString()
    }

    @SuppressLint("ClickableViewAccessibility")
    fun destroy() {
        webView.removeOnLayoutChangeListener(layoutListener)
        excludeView?.removeOnLayoutChangeListener(layoutListener)
        webView.setOnTouchListener(null)
        webView.setOnScrollChangeListener(null as View.OnScrollChangeListener?)
        controller.close()
        initialized = false
    }
}

/** Pure rendering — no WebView dependency. Strokes are in document coords. */
internal fun renderStrokesToPng(
    width: Int,
    height: Int,
    strokes: List<Stroke>,
    transform: ViewTransform = ViewTransform(),
): ByteArray {
    val bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
    val canvas = Canvas(bitmap)
    canvas.drawColor(Color.TRANSPARENT)

    val paint = Paint().apply {
        color = Color.BLACK
        style = Paint.Style.STROKE
        isAntiAlias = true
        strokeCap = Paint.Cap.ROUND
        strokeJoin = Paint.Join.ROUND
    }

    for (stroke in strokes) {
        if (stroke.points.size < 2) continue
        paint.strokeWidth = transform.docToScreenWidth(stroke.width)
        var prevX = transform.docToScreenX(stroke.points[0].first)
        var prevY = transform.docToScreenY(stroke.points[0].second)
        for (i in 1 until stroke.points.size) {
            val curX = transform.docToScreenX(stroke.points[i].first)
            val curY = transform.docToScreenY(stroke.points[i].second)
            canvas.drawLine(prevX, prevY, curX, curY, paint)
            prevX = curX
            prevY = curY
        }
    }

    val out = ByteArrayOutputStream()
    bitmap.compress(Bitmap.CompressFormat.PNG, 100, out)
    bitmap.recycle()
    return out.toByteArray()
}
