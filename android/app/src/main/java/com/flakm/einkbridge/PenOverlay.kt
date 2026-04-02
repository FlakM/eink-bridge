package com.flakm.einkbridge

import android.annotation.SuppressLint
import android.app.AlertDialog
import android.graphics.*
import android.graphics.Rect
import android.util.Log
import android.view.MotionEvent
import android.view.View
import android.webkit.WebView
import com.onyx.android.sdk.api.device.epd.EpdController
import com.onyx.android.sdk.data.note.TouchPoint
import com.onyx.android.sdk.pen.RawInputCallback
import com.onyx.android.sdk.pen.TouchHelper
import com.onyx.android.sdk.pen.data.TouchPointList
import kotlinx.coroutines.CoroutineScope
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
    private val onStrokeCommitted: (() -> Unit)? = null,
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
                onStrokeCommitted?.invoke()
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
    /** Called whenever committed strokes change (draw, undo, clear, erase). */
    private val onStrokesChanged: (() -> Unit)? = null,
    internal var ocrManager: OcrManager? = null,
) {
    private val controller: PenInputController by lazy {
        val c = controllerOverride ?: OnyxPenController(buf, ::currentTransform, onEraseApplied = {
            undoStack.clear()
            notifyStrokeView()
            controller.resetRenderBuffer()
            onStrokesChanged?.invoke()
            webView.post { refreshStrokeLinks() }
        }, onStrokeProgress = { notifyStrokeViewLive() }, onStrokeCommitted = {
            undoStack.add(UndoAction.StrokeAdded(buf.size - 1))
            onStrokesChanged?.invoke()
            webView.post { refreshStrokeLinks() }
            scheduleOcr()
        })
        if (c is OnyxPenController && c.onStrokeProgress == null) {
            c.onStrokeProgress = { notifyStrokeViewLive() }
        }
        c
    }
    private var initialized = false
    private val ocrResults = mutableListOf<OcrResult>()

    var annotationMode = false
        set(value) {
            field = value
            notifyStrokeView()
        }

    private fun scheduleOcr() {
        ocrManager?.schedule(_bindGroups.toList(), buf.strokes)
    }

    internal fun onGroupOcrResult(groupId: Int, text: String) {
        val idx = _bindGroups.indexOfFirst { it.id == groupId }
        if (idx < 0) return
        _bindGroups[idx] = _bindGroups[idx].copy(recognizedText = text)
        notifyStrokeView()
        onBindGroupsChanged?.invoke()
    }

    internal fun onUnboundOcrResults(results: List<OcrResult>) {
        ocrResults.clear()
        ocrResults.addAll(results)
        notifyStrokeView()
        onOcrResultsChanged?.invoke(ocrResults.toList())
    }

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
            if (!isBindMode && !isSelectMode) {
                if (!action) {
                    notifyStrokeView()
                    controller.setEnabled(false)
                } else {
                    controller.resetRenderBuffer()
                    notifyStrokeView()
                }
            }
            isBindMode || isSelectMode  // consume finger events to prevent WebView scroll
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
        setupScaleTracker()
        maybeInit()
        if (!buf.isEmpty) notifyStrokeView()
    }

    private fun setupScaleTracker() {
        val origListener = touchRouter

        @SuppressLint("ClickableViewAccessibility")
        val wrappedListener = View.OnTouchListener { v, event ->
            val result = origListener.onTouch(v, event)

            when {
                isSelectMode && event.pointerCount == 1 -> when (event.actionMasked) {
                    MotionEvent.ACTION_DOWN -> {
                        val hit = strokeView?.hitTestLabel(event.x, event.y)
                        if (hit != null) {
                            selectedLabel = hit
                            dragLabel = hit
                            dragStartX = event.x
                            dragStartY = event.y
                            dragBaseDocX = when (hit) {
                                is LabelId.Group -> groupLabelOffsets[hit.groupId]?.first ?: 0f
                                is LabelId.Cluster -> clusterLabelOffsets[hit.clusterIdx]?.first ?: 0f
                            }
                            dragBaseDocY = when (hit) {
                                is LabelId.Group -> groupLabelOffsets[hit.groupId]?.second ?: 0f
                                is LabelId.Cluster -> clusterLabelOffsets[hit.clusterIdx]?.second ?: 0f
                            }
                        } else {
                            selectedLabel = null
                            dragLabel = null
                        }
                        notifyStrokeView()
                    }
                    MotionEvent.ACTION_MOVE -> {
                        val dragging = dragLabel
                        if (dragging != null) {
                            val t = currentTransform()
                            val docDX = (event.x - dragStartX) / t.scale + dragBaseDocX
                            val docDY = (event.y - dragStartY) / t.scale + dragBaseDocY
                            when (dragging) {
                                is LabelId.Group -> groupLabelOffsets[dragging.groupId] = docDX to docDY
                                is LabelId.Cluster -> clusterLabelOffsets[dragging.clusterIdx] = docDX to docDY
                            }
                            notifyStrokeView()
                        }
                    }
                    MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                        dragLabel = null
                    }
                }
                isBindMode && event.pointerCount == 1 -> when (event.actionMasked) {
                    MotionEvent.ACTION_DOWN -> {
                        val pts = mutableListOf(PointF(event.x, event.y))
                        bindPoints = pts
                        strokeView?.setBindPath(pts)
                        strokeView?.bindDrawingActive = true
                        strokeView?.invalidate()
                        webView.evaluateJavascript("window.__einkHighlightAll && window.__einkHighlightAll()", null)
                    }
                    MotionEvent.ACTION_MOVE -> {
                        bindPoints?.add(PointF(event.x, event.y))
                        strokeView?.setBindPath(bindPoints)
                    }
                    MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                        val pts = bindPoints ?: emptyList<PointF>()
                        bindPoints = null
                        strokeView?.setBindPath(null)
                        strokeView?.bindDrawingActive = false
                        strokeView?.invalidate()
                        webView.evaluateJavascript("window.__einkUnhighlightAll && window.__einkUnhighlightAll()", null)
                        val moved = pts.size > 1 && run {
                            val dx = pts.last().x - pts.first().x
                            val dy = pts.last().y - pts.first().y
                            dx * dx + dy * dy > TAP_THRESHOLD_PX2
                        }
                        if (moved && event.actionMasked == MotionEvent.ACTION_UP) {
                            completeBindGesture(pts)
                        } else {
                            flashMarkerNear(event.x, event.y)
                        }
                    }
                }
                !isBindMode && !isSelectMode && event.actionMasked == MotionEvent.ACTION_UP -> {
                    if (!showOcrPopupNear(event.x, event.y)) flashMarkerNear(event.x, event.y)
                }
                event.actionMasked == MotionEvent.ACTION_MOVE && event.pointerCount >= 2 ->
                    strokeView?.updateTransform(currentTransform())
            }
            // In select mode with a single finger always consume to prevent WebView scroll
            result || (isSelectMode && event.pointerCount == 1)
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
    fun setStrokeColor(color: Int) { buf.setColor(color) }
    fun setStylePencil() { controller.setStylePencil() }
    fun setStyleBrush() { controller.setStyleBrush() }
    fun setStyleEraser() { controller.setStyleEraser() }

    fun undoLastStroke() {
        if (undoStack.isNotEmpty()) {
            when (val action = undoStack.removeAt(undoStack.lastIndex)) {
                is UndoAction.StrokeAdded -> {
                    buf.undo()
                    // Fix up bind groups that reference removed stroke
                    val removedIdx = action.strokeIndex
                    _bindGroups.removeAll { it.strokeIndices.contains(removedIdx) && it.strokeIndices.size == 1 }
                    _bindGroups.replaceAll { g ->
                        if (removedIdx in g.strokeIndices) {
                            g.copy(strokeIndices = g.strokeIndices - removedIdx)
                        } else g
                    }
                }
                is UndoAction.BindGroupAdded -> {
                    _bindGroups.removeAll { it.id == action.groupId }
                    onBindGroupsChanged?.invoke()
                    syncBindGroupsToWebView()
                }
            }
        } else {
            buf.undo()
        }
        notifyStrokeView()
        controller.resetRenderBuffer()
        onStrokesChanged?.invoke()
        refreshStrokeLinks()
    }

    fun clearStrokes() {
        buf.clear()
        undoStack.clear()
        ocrResults.clear()
        ocrManager?.clearCache()
        notifyStrokeView()
        controller.resetRenderBuffer()
        onStrokesChanged?.invoke()
        webView.post {
            webView.evaluateJavascript("window.__einkComputeStrokeLinks && window.__einkComputeStrokeLinks([],[])", null)
        }
    }

    private fun notifyStrokeView() {
        strokeView?.update(
            buf.strokes, currentTransform(), _bindGroups, bindPoints?.toList(),
            ocrResults.toList(), annotationMode,
            groupLabelOffsets.toMap(), clusterLabelOffsets.toMap(),
            selectedLabel,
        )
    }

    private fun notifyStrokeViewLive() {
        strokeView?.update(buf.allStrokes(), currentTransform())
    }

    fun enterBindMode() {
        isBindMode = true
        controller.setEnabled(false)
    }

    fun exitBindMode() {
        isBindMode = false
        bindPoints = null
        strokeView?.setBindPath(null)
        controller.resetRenderBuffer()
    }

    fun enterSelectMode() {
        isSelectMode = true
        controller.setEnabled(false)
    }

    fun exitSelectMode() {
        isSelectMode = false
        dragLabel = null
        selectedLabel = null
        controller.resetRenderBuffer()
        notifyStrokeView()
    }

    private fun completeBindGesture(pts: List<PointF>) {
        val t = currentTransform()
        val docPolygon = pts.map { t.screenToDocX(it.x) to t.screenToDocY(it.y) }
        val docXs = docPolygon.map { it.first }
        val docYs = docPolygon.map { it.second }
        val left = docXs.min()
        val top = docYs.min()
        val right = docXs.max()
        val bottom = docYs.max()

        val strokeIndices = mutableSetOf<Int>()
        buf.strokes.forEachIndexed { idx, stroke ->
            val (cx, cy) = strokeCentroid(stroke)
            val captured = pointInPolygon(cx, cy, docPolygon)
                || stroke.points.any { (px, py) -> pointInPolygon(px, py, docPolygon) }
            if (captured) strokeIndices.add(idx)
        }

        webView.evaluateJavascript("window.__einkFindElements($left, $top, $right, $bottom)") { raw ->
            val elements = parseFoundElements(raw)
            val markerX = docXs.average().toFloat()
            val markerY = docYs.average().toFloat()
            if (strokeIndices.isNotEmpty() || elements.isNotEmpty()) {
                val color = nextGroupColor()
                val centers = strokeIndices.mapNotNull { idx ->
                    buf.strokes.getOrNull(idx)?.let { strokeCentroid(it) }
                }
                val elementCenters = elements.map { it.cx to it.cy }
                val allCenters = centers + elementCenters
                val midX = if (allCenters.isNotEmpty()) allCenters.map { it.first }.average().toFloat() else markerX
                val midY = if (allCenters.isNotEmpty()) allCenters.map { it.second }.average().toFloat() else markerY
                val groupId = nextGroupId++
                _bindGroups.add(BindGroup(
                    id = groupId,
                    color = color,
                    strokeIndices = strokeIndices,
                    elementIndices = elements.map { it.i },
                    elementRefs = elements.map { ElementRef(it.section ?: it.id, it.tag, it.text) },
                    markerDocX = midX,
                    markerDocY = midY,
                    strokeDocCenters = centers,
                    elementDocCenters = elementCenters,
                ))
                @Suppress("DEPRECATION")
                (webView.context.getSystemService(android.content.Context.VIBRATOR_SERVICE) as? android.os.Vibrator)?.vibrate(50)
                undoStack.add(UndoAction.BindGroupAdded(groupId))
                notifyStrokeView()
                syncBindGroupsToWebView()
                expandGroup(_bindGroups.last())
            } else {
                notifyStrokeView()
            }
            onBindGroupsChanged?.invoke()
            onBindComplete?.invoke()
        }
    }

    private fun parseFoundElements(raw: String?): List<FoundElement> {
        val cleaned = raw?.trim()?.removeSurrounding("\"")
            ?.replace("\\\"", "\"")?.replace("\\\\", "\\") ?: "[]"
        return try {
            val arr = org.json.JSONArray(cleaned)
            (0 until arr.length()).map {
                val obj = arr.getJSONObject(it)
                FoundElement(
                    i = obj.getInt("i"),
                    tag = obj.getString("tag"),
                    id = obj.optString("id", null),
                    section = obj.optString("section", null),
                    text = obj.optString("text", ""),
                    cx = obj.optDouble("cx", 0.0).toFloat(),
                    cy = obj.optDouble("cy", 0.0).toFloat(),
                )
            }
        } catch (_: Exception) { emptyList<FoundElement>() }
    }

    var onDeleteBindGroup: ((BindGroup) -> Unit)? = null

    private fun ocrTextNear(screenX: Float, screenY: Float): String? {
        val t = currentTransform()
        val tapRadius = StrokeView.BADGE_RADIUS_PX * 2.5f
        for (group in _bindGroups) {
            val text = group.recognizedText ?: continue
            if (StrokeView.groupScreenDiagonal(group, buf.strokes, t) < StrokeView.MIN_OCR_DIAGONAL_PX) continue
            val sx = t.docToScreenX(group.markerDocX)
            val sy = t.docToScreenY(group.markerDocY) - StrokeView.MARKER_RADIUS_PX - StrokeView.BADGE_RADIUS_PX - 6f
            val dx = sx - screenX; val dy = sy - screenY
            if (dx * dx + dy * dy <= tapRadius * tapRadius) return text
        }
        for (result in ocrResults) {
            val sx = t.docToScreenX(result.docX)
            val sy = t.docToScreenY(result.docY) - StrokeView.MARKER_RADIUS_PX - StrokeView.BADGE_RADIUS_PX - 6f
            val dx = sx - screenX; val dy = sy - screenY
            if (dx * dx + dy * dy <= tapRadius * tapRadius) return result.text
        }
        return null
    }

    private fun showOcrPopupNear(screenX: Float, screenY: Float): Boolean {
        val text = ocrTextNear(screenX, screenY) ?: return false
        AlertDialog.Builder(webView.context)
            .setTitle("Recognized text")
            .setMessage(text)
            .setPositiveButton("OK", null)
            .show()
        return true
    }

    private fun flashMarkerNear(screenX: Float, screenY: Float) {
        if (_bindGroups.isEmpty()) return
        val t = currentTransform()
        val docX = t.screenToDocX(screenX)
        val docY = t.screenToDocY(screenY)
        val thresholdDoc = StrokeView.MARKER_RADIUS_PX * 3f / t.scale
        val group = _bindGroups.minByOrNull { g ->
            val dx = g.markerDocX - docX; val dy = g.markerDocY - docY; dx * dx + dy * dy
        } ?: return
        val dx = group.markerDocX - docX; val dy = group.markerDocY - docY
        if (dx * dx + dy * dy > thresholdDoc * thresholdDoc) return

        val alreadyExpanded = strokeView?.isGroupExpanded(group.id) == true
        if (alreadyExpanded) {
            onDeleteBindGroup?.invoke(group)
        } else {
            expandGroup(group)
        }
    }

    private fun expandGroup(group: BindGroup) {
        strokeView?.showGroup(group.id)
        val colorHex = "#%06X".format(group.color and 0xFFFFFF)
        val indicesJson = org.json.JSONArray(group.elementIndices).toString()
        webView.evaluateJavascript("window.__einkFlashGroup($indicesJson, '$colorHex')", null)
    }

    fun removeBindGroup(groupId: Int) {
        _bindGroups.removeAll { it.id == groupId }
        undoStack.removeAll { it is UndoAction.BindGroupAdded && it.groupId == groupId }
        notifyStrokeView()
        syncBindGroupsToWebView()
        onBindGroupsChanged?.invoke()
    }

    private fun nextGroupColor(): Int {
        val used = _bindGroups.map { it.color }.toSet()
        return GROUP_PALETTE.firstOrNull { it !in used } ?: GROUP_PALETTE[_bindGroups.size % GROUP_PALETTE.size]
    }

    internal sealed class UndoAction {
        data class StrokeAdded(val strokeIndex: Int) : UndoAction()
        data class BindGroupAdded(val groupId: Int) : UndoAction()
    }

    private val _bindGroups = mutableListOf<BindGroup>()
    val bindGroups: List<BindGroup> get() = _bindGroups
    private val undoStack = mutableListOf<UndoAction>()
    private var isBindMode = false
    private var bindPoints: MutableList<PointF>? = null
    private var nextGroupId = 0
    var onBindComplete: (() -> Unit)? = null
    var onBindGroupsChanged: (() -> Unit)? = null
    var onOcrResultsChanged: ((List<OcrResult>) -> Unit)? = null

    // Select / move mode
    private var isSelectMode = false
    private val groupLabelOffsets = mutableMapOf<Int, Pair<Float, Float>>()
    private val clusterLabelOffsets = mutableMapOf<Int, Pair<Float, Float>>()
    private var selectedLabel: LabelId? = null
    private var dragLabel: LabelId? = null
    private var dragStartX = 0f
    private var dragStartY = 0f
    private var dragBaseDocX = 0f
    private var dragBaseDocY = 0f

    fun loadOcrResults(results: List<OcrResult>) {
        ocrResults.clear()
        ocrResults.addAll(results)
        notifyStrokeView()
    }

    fun loadBindGroups(groups: List<BindGroup>) {
        _bindGroups.clear()
        _bindGroups.addAll(groups)
        nextGroupId = (groups.maxOfOrNull { it.id } ?: -1) + 1
        notifyStrokeView()
        syncBindGroupsToWebView()
    }

    fun queryElementMap(callback: (List<ElementEntry>) -> Unit) {
        webView.evaluateJavascript("JSON.stringify(window.__einkElementMap || [])") { json ->
            val cleaned = json?.trim()?.removeSurrounding("\"")
                ?.replace("\\\"", "\"")
                ?.replace("\\\\", "\\") ?: "[]"
            try {
                callback(parseElementMap(cleaned))
            } catch (_: Exception) {
                callback(emptyList())
            }
        }
    }

    fun clearBindGroups() {
        _bindGroups.clear()
        undoStack.removeAll { it is UndoAction.BindGroupAdded }
        onBindGroupsChanged?.invoke()
        notifyStrokeView()
        syncBindGroupsToWebView()
    }

    fun refreshStrokeLinks() {
        webView.post {
            webView.evaluateJavascript("window.__einkComputeStrokeLinks && window.__einkComputeStrokeLinks([],[])", null)
        }
    }

    private fun syncBindGroupsToWebView() {
        if (_bindGroups.isEmpty()) {
            webView.post {
                webView.evaluateJavascript("window.__einkApplyBindGroups && window.__einkApplyBindGroups([])", null)
            }
            return
        }
        val groupsJson = org.json.JSONArray().apply {
            for (g in _bindGroups) {
                put(org.json.JSONObject().apply {
                    put("color", "#%06X".format(g.color and 0xFFFFFF))
                    put("indices", org.json.JSONArray(g.elementIndices))
                })
            }
        }
        webView.post {
            webView.evaluateJavascript("window.__einkApplyBindGroups && window.__einkApplyBindGroups($groupsJson)", null)
        }
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

    companion object {
        private const val TAP_THRESHOLD_PX2 = 30f * 30f
        private val GROUP_PALETTE = intArrayOf(
            0xFFe74c3c.toInt(), 0xFF2196f3.toInt(), 0xFF4caf50.toInt(),
            0xFFff9800.toInt(), 0xFF9c27b0.toInt(), 0xFF009688.toInt(),
        )
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
