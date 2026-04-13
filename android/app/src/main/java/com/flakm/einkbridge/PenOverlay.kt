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
import com.onyx.android.sdk.api.device.epd.UpdateMode
import com.onyx.android.sdk.data.note.TouchPoint
import com.onyx.android.sdk.pen.RawInputCallback
import com.onyx.android.sdk.pen.TouchHelper
import com.onyx.android.sdk.pen.data.TouchPointList
import kotlinx.coroutines.CoroutineScope
import org.json.JSONArray
import org.json.JSONObject
import java.io.ByteArrayOutputStream

/**
 * The high-level user-facing tool mode.
 *
 * Pen drawing is enabled only in [DRAW]; [TAG] and [MOVE] are gesture-driven modes
 * that consume finger events and must keep the Onyx SDK disabled to avoid accidental strokes.
 */
enum class ToolMode { DRAW, TAG, MOVE }

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
    private val onEraseApplied: (Set<Int>) -> Unit = {},
    internal var onStrokeProgress: (() -> Unit)? = null,
    private val onStrokeCommitted: (() -> Unit)? = null,
    internal var onErasePath: ((path: List<Pair<Float, Float>>, transform: ViewTransform) -> Unit)? = null,
    /**
     * Called after a stroke/erase ends to request the SDK be re-enabled.
     * Routed through [PenOverlay.applyDrawingState] so a mode change during the
     * 50 ms cooldown suppresses the re-enable instead of leaving the SDK on in bind/select mode.
     */
    internal var requestReEnable: (() -> Unit)? = null,
) : PenInputController {
    private var touchHelper: TouchHelper? = null
    private var penViewRef: java.lang.ref.WeakReference<View>? = null
    private var pendingWidth: Float? = null
    private var eraseMode = false
    private var eraserPath = mutableListOf<Pair<Float, Float>>()
    private var currentWidth = 3f
    private var moveCount = 0
    private var lastLimitRect: Rect? = null
    private var lastExcludeRects: List<Rect> = emptyList()
    private var currentStyle: Int = TouchHelper.STROKE_STYLE_PENCIL
    private var pendingReEnable: Runnable? = null

    private fun setEpdFastDraw() {
        val v = penViewRef?.get() ?: return
        try { EpdController.setViewDefaultUpdateMode(v, UpdateMode.DU) } catch (_: Exception) {}
    }
    private fun setEpdFullColor() {
        val v = penViewRef?.get() ?: return
        try { EpdController.setViewDefaultUpdateMode(v, UpdateMode.GC) } catch (_: Exception) {}
    }

    private val callback = object : RawInputCallback() {
        override fun onBeginRawDrawing(b: Boolean, tp: TouchPoint) {
            moveCount = 0
            setEpdFastDraw()
            Log.d(DRAW_TAG, "onBeginRawDrawing eraseMode=$eraseMode tp=(${tp.x},${tp.y}) bufSize=${buf.size}")
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
            Log.d(DRAW_TAG, "onEndRawDrawing eraseMode=$eraseMode bufSize=${buf.size} movePoints=$moveCount")
            if (eraseMode) {
                eraserPath.add(tp.x to tp.y)
                val t = getTransform()
                val path = eraserPath.toList()
                val removed = buf.erase(path, ERASER_RADIUS, t)
                eraserPath.clear()
                onErasePath?.invoke(path, t)
                onEraseApplied(removed)
            } else {
                buf.commit()
                onStrokeProgress?.invoke()
                onStrokeCommitted?.invoke()
            }
            scheduleReEnableAfterCooldown()
            setEpdFullColor()
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
            setEpdFastDraw()
            eraserPath = mutableListOf(tp.x to tp.y)
        }
        override fun onRawErasingTouchPointMoveReceived(tp: TouchPoint) {
            eraserPath.add(tp.x to tp.y)
        }
        override fun onRawErasingTouchPointListReceived(list: TouchPointList) {}
        override fun onEndRawErasing(b: Boolean, tp: TouchPoint) {
            eraserPath.add(tp.x to tp.y)
            val t = getTransform()
            val path = eraserPath.toList()
            val removed = buf.erase(path, ERASER_RADIUS, t)
            eraserPath.clear()
            onErasePath?.invoke(path, t)
            onEraseApplied(removed)
            scheduleReEnableAfterCooldown()
            setEpdFullColor()
        }
    }

    /**
     * After a stroke or erase ends, disable raw drawing briefly so finger taps on
     * the toolbar aren't blocked, then ask the overlay whether it should be re-enabled.
     * Routing the re-enable through [requestReEnable] lets the overlay suppress it
     * if a mode change happened during the cooldown.
     */
    private fun scheduleReEnableAfterCooldown() {
        val helper = touchHelper ?: return
        Log.d(DRAW_TAG, "scheduleReEnableAfterCooldown: setRawDrawingEnabled(false) + post 50ms")
        helper.setRawDrawingEnabled(false)
        val view = penViewRef?.get() ?: return
        pendingReEnable?.let { view.removeCallbacks(it) }
        val runnable = Runnable {
            pendingReEnable = null
            Log.d(DRAW_TAG, "cooldown runnable fired: invoking requestReEnable")
            requestReEnable?.invoke() ?: touchHelper?.setRawDrawingEnabled(true)
        }
        pendingReEnable = runnable
        view.postDelayed(runnable, 50)
    }

    override fun open(view: View, limitRect: Rect, excludeRects: List<Rect>) {
        val helper = TouchHelper.create(view, callback)
        touchHelper = helper
        penViewRef = java.lang.ref.WeakReference(view)
        lastLimitRect = Rect(limitRect)
        lastExcludeRects = excludeRects.toList()
        currentStyle = TouchHelper.STROKE_STYLE_PENCIL
        helper.setStrokeWidth(currentWidth)
            .setLimitRect(limitRect, excludeRects)
            .openRawDrawing()
        helper.setRawDrawingRenderEnabled(false)
        helper.setStrokeStyle(currentStyle)
        helper.setRawDrawingEnabled(true)
    }

    override fun updateLimitRect(limitRect: Rect, excludeRects: List<Rect>) {
        lastLimitRect = Rect(limitRect)
        lastExcludeRects = excludeRects.toList()
        touchHelper?.setLimitRect(limitRect, excludeRects)
    }

    override fun setEnabled(enabled: Boolean) {
        Log.d(DRAW_TAG, "setEnabled($enabled)")
        if (!enabled) cancelPendingReEnable()
        touchHelper?.setRawDrawingEnabled(enabled)
    }
    override fun setStrokeWidth(width: Float) {
        pendingWidth = width
        currentWidth = width
    }
    override fun setStylePencil() {
        eraseMode = false
        currentStyle = TouchHelper.STROKE_STYLE_PENCIL
        touchHelper?.setStrokeStyle(currentStyle)
    }
    override fun setStyleBrush() {
        eraseMode = false
        currentStyle = TouchHelper.STROKE_STYLE_FOUNTAIN
        touchHelper?.setStrokeStyle(currentStyle)
    }
    override fun setStyleEraser() { eraseMode = true }

    override fun resetRenderBuffer() {
        val helper = touchHelper ?: return
        Log.w(DRAW_TAG, "resetRenderBuffer: CLOSE+REOPEN raw drawing", Throwable("reset stacktrace"))
        cancelPendingReEnable()
        helper.setRawDrawingEnabled(false)
        helper.closeRawDrawing()
        // Re-apply all state that closeRawDrawing may have dropped.
        lastLimitRect?.let { helper.setLimitRect(it, lastExcludeRects) }
        helper.setStrokeWidth(currentWidth)
        helper.openRawDrawing()
        helper.setRawDrawingRenderEnabled(false)
        helper.setStrokeStyle(currentStyle)
        helper.setRawDrawingEnabled(true)
    }

    override fun close() {
        cancelPendingReEnable()
        touchHelper?.setRawDrawingEnabled(false)
        touchHelper?.closeRawDrawing()
        touchHelper = null
    }

    private fun cancelPendingReEnable() {
        val runnable = pendingReEnable ?: return
        penViewRef?.get()?.removeCallbacks(runnable)
        pendingReEnable = null
    }

    companion object {
        const val ERASER_RADIUS = 30f
        const val DRAW_TAG = "EinkDraw"
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
    private val bridge = WebViewBridge(webView)
    private val controller: PenInputController by lazy {
        val c = controllerOverride ?: OnyxPenController(buf, ::currentTransform, onEraseApplied = { removedIndices ->
            if (removedIndices.isNotEmpty()) {
                _bindGroups.removeAll { bg -> bg.strokeIndices.all { it in removedIndices } }
                _bindGroups.replaceAll { bg ->
                    val newIndices = bg.strokeIndices.mapNotNullTo(mutableSetOf()) { idx ->
                        if (idx in removedIndices) null
                        else idx - removedIndices.count { it < idx }
                    }
                    val strokesRemoved = newIndices.size < bg.strokeIndices.size
                    bg.copy(
                        strokeIndices = newIndices,
                        recognizedText = if (strokesRemoved) null else bg.recognizedText,
                    )
                }
                // Drop OCR results that had any strokes erased — text is now stale.
                ocrResults.removeAll { r -> r.strokeIndices.any { it in removedIndices } }
                val reindexed = ocrResults.map { r ->
                    r.copy(strokeIndices = r.strokeIndices.mapTo(mutableSetOf()) { idx ->
                        idx - removedIndices.count { it < idx }
                    })
                }
                ocrResults.clear()
                ocrResults.addAll(reindexed)
            }
            undoStack.clear()
            notifyStrokeView()
            controller.resetRenderBuffer()
            onStrokesChanged?.invoke()
            onBindGroupsChanged?.invoke()
            onOcrResultsChanged?.invoke(ocrResults.toList())
            webView.post { refreshStrokeLinks() }
            scheduleOcr()
        }, onStrokeProgress = { notifyStrokeViewLive() }, onStrokeCommitted = {
            lastStrokeEndMs = System.currentTimeMillis()
            val newIdx = buf.size - 1
            undoStack.add(UndoAction.StrokeAdded(newIdx))
            tryAbsorbStrokeIntoGroup(newIdx)
            notifyStrokeView()
            onStrokesChanged?.invoke()
            webView.post { refreshStrokeLinks() }
            scheduleOcr()
        })
        if (c is OnyxPenController && c.onStrokeProgress == null) {
            c.onStrokeProgress = { notifyStrokeViewLive() }
        }
        if (c is OnyxPenController && c.onErasePath == null) {
            c.onErasePath = { path, transform -> eraseBindGroupsNearPath(path, transform) }
        }
        if (c is OnyxPenController) {
            c.requestReEnable = { applyDrawingState() }
        }
        c
    }
    private var initialized = false
    private val ocrResults = mutableListOf<OcrResult>()
    private var lastStrokeEndMs = 0L
    private var fingerDown = false
    private var paused = false

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
        val toolTypes = (0 until event.pointerCount).map { event.getToolType(it) }
        if (action != null) {
            // action == false → finger DOWN, action == true → finger UP/CANCEL
            fingerDown = !action
            Log.d(
                "EinkDraw",
                "touchRouter FINGER actionMasked=${event.actionMasked} fingerDown=$fingerDown " +
                    "pointerCount=${event.pointerCount} toolTypes=$toolTypes"
            )
            applyDrawingState()
            notifyStrokeView()
            isBindMode || isSelectMode  // consume finger events to prevent WebView scroll
        } else {
            // Stylus/eraser event — consume it so the WebView doesn't scroll.
            val hasPen = (0 until event.pointerCount).any {
                val t = event.getToolType(it)
                t == MotionEvent.TOOL_TYPE_STYLUS || t == MotionEvent.TOOL_TYPE_ERASER
            }
            if (event.actionMasked == MotionEvent.ACTION_DOWN || event.actionMasked == MotionEvent.ACTION_UP) {
                Log.d(
                    "EinkDraw",
                    "touchRouter STYLUS actionMasked=${event.actionMasked} hasPen=$hasPen " +
                        "toolTypes=$toolTypes bufSize=${buf.size}"
                )
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
                        val labelHit = strokeView?.hitTestLabel(event.x, event.y)
                        if (labelHit != null) {
                            // Tapped on a label — move label only
                            selectedLabel = labelHit
                            dragLabel = labelHit
                            dragGroupId = null
                            dragStartX = event.x
                            dragStartY = event.y
                            dragBaseDocX = when (labelHit) {
                                is LabelId.Group -> groupLabelOffsets[labelHit.groupId]?.first ?: 0f
                                is LabelId.Cluster -> clusterLabelOffsets[labelHit.clusterIdx]?.first ?: 0f
                            }
                            dragBaseDocY = when (labelHit) {
                                is LabelId.Group -> groupLabelOffsets[labelHit.groupId]?.second ?: 0f
                                is LabelId.Cluster -> clusterLabelOffsets[labelHit.clusterIdx]?.second ?: 0f
                            }
                        } else {
                            val groupHit = strokeView?.hitTestGroup(event.x, event.y)
                            val clusterHit = if (groupHit == null) strokeView?.hitTestCluster(event.x, event.y) else null
                            when {
                                groupHit != null -> {
                                    selectedLabel = LabelId.Group(groupHit)
                                    dragGroupId = groupHit; dragClusterIdx = null; dragLabel = null
                                    dragStartX = event.x; dragStartY = event.y
                                    dragBaseDocX = groupStrokeOffsets[groupHit]?.first ?: 0f
                                    dragBaseDocY = groupStrokeOffsets[groupHit]?.second ?: 0f
                                    onGroupSelectionChanged?.invoke(groupHit)
                                }
                                clusterHit != null -> {
                                    selectedLabel = LabelId.Cluster(clusterHit)
                                    dragClusterIdx = clusterHit; dragGroupId = null; dragLabel = null
                                    dragStartX = event.x; dragStartY = event.y
                                    dragBaseDocX = clusterStrokeOffsets[clusterHit]?.first ?: 0f
                                    dragBaseDocY = clusterStrokeOffsets[clusterHit]?.second ?: 0f
                                    onGroupSelectionChanged?.invoke(null)
                                }
                                else -> {
                                    selectedLabel = null; dragLabel = null
                                    dragGroupId = null; dragClusterIdx = null
                                    onGroupSelectionChanged?.invoke(null)
                                    onSelectModeExitRequested?.invoke()
                                }
                            }
                        }
                        notifyStrokeView()
                    }
                    MotionEvent.ACTION_MOVE -> {
                        val t = currentTransform()
                        val docDX = (event.x - dragStartX) / t.scale
                        val docDY = (event.y - dragStartY) / t.scale
                        when {
                            dragLabel != null -> {
                                when (val d = dragLabel!!) {
                                    is LabelId.Group -> groupLabelOffsets[d.groupId] = dragBaseDocX + docDX to dragBaseDocY + docDY
                                    is LabelId.Cluster -> clusterLabelOffsets[d.clusterIdx] = dragBaseDocX + docDX to dragBaseDocY + docDY
                                }
                                notifyStrokeView()
                            }
                            dragGroupId != null -> {
                                groupStrokeOffsets[dragGroupId!!] = dragBaseDocX + docDX to dragBaseDocY + docDY
                                notifyStrokeView()
                            }
                            dragClusterIdx != null -> {
                                clusterStrokeOffsets[dragClusterIdx!!] = dragBaseDocX + docDX to dragBaseDocY + docDY
                                notifyStrokeView()
                            }
                        }
                    }
                    MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                        if (event.actionMasked == MotionEvent.ACTION_UP) {
                            when {
                                dragClusterIdx != null -> commitClusterOffset(dragClusterIdx!!)
                                dragGroupId != null -> commitGroupOffset(dragGroupId!!)
                            }
                        }
                        dragLabel = null; dragGroupId = null; dragClusterIdx = null
                    }
                }
                isBindMode && event.pointerCount == 1 -> when (event.actionMasked) {
                    MotionEvent.ACTION_DOWN -> {
                        bindHandler.onDown(event.x, event.y)
                        strokeView?.setBindPath(bindHandler.currentPoints())
                        strokeView?.bindDrawingActive = true
                        strokeView?.invalidate()
                        bridge.highlightAll()
                    }
                    MotionEvent.ACTION_MOVE -> {
                        bindHandler.onMove(event.x, event.y)
                        strokeView?.setBindPath(bindHandler.currentPoints())
                    }
                    MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                        strokeView?.setBindPath(null)
                        strokeView?.bindDrawingActive = false
                        strokeView?.invalidate()
                        bridge.unhighlightAll()
                        bindHandler.onUp(event.x, event.y, event.actionMasked == MotionEvent.ACTION_CANCEL)
                    }
                }
                !isBindMode && !isSelectMode && event.actionMasked == MotionEvent.ACTION_UP -> {
                    if (!showOcrPopupNear(event.x, event.y)) flashMarkerNear(event.x, event.y)
                }
                event.actionMasked == MotionEvent.ACTION_MOVE && event.pointerCount >= 2 ->
                    strokeView?.updateTransform(currentTransform())
            }
            // Block scroll briefly after a stroke ends to absorb residual touch events.
            val scrollBlocked = !isBindMode && !isSelectMode &&
                System.currentTimeMillis() - lastStrokeEndMs < SCROLL_BLOCK_MS
            result || scrollBlocked || (isSelectMode && event.pointerCount == 1)
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

    /** Lifecycle hook — call from Activity.onResume. */
    fun onResumed() {
        paused = false
        if (!initialized) initController()
        applyDrawingState()
    }

    /** Lifecycle hook — call from Activity.onPause. */
    fun onPaused() {
        paused = true
        applyDrawingState()
    }

    /**
     * Recomputes whether raw drawing should be enabled and pushes the result to the controller.
     * Single source of truth: drawing is enabled iff we are in [ToolMode.DRAW], no finger is down,
     * and the activity is resumed.
     *
     * [reset] = true does a full close+reopen of the raw drawing layer to clear any ghost
     * stylus/finger paint. Only pass true on events that can leave stale marks on the SDK's
     * render layer (finger UP, mode exit, undo, clear). The post-stroke cooldown MUST pass false
     * — close+reopen between strokes causes the Onyx SDK to drop just-committed strokes from the
     * visible layer, making new letters vanish mid-word.
     */
    private fun applyDrawingState(reset: Boolean = false) {
        if (!initialized) {
            Log.d("EinkDraw", "applyDrawingState SKIP initialized=false")
            return
        }
        val shouldDraw = !isBindMode && !isSelectMode && !fingerDown && !paused
        Log.d(
            "EinkDraw",
            "applyDrawingState reset=$reset shouldDraw=$shouldDraw " +
                "isBindMode=$isBindMode isSelectMode=$isSelectMode fingerDown=$fingerDown paused=$paused " +
                "bufSize=${buf.size}"
        )
        if (!shouldDraw) {
            controller.setEnabled(false)
        } else if (reset) {
            controller.resetRenderBuffer()
        } else {
            controller.setEnabled(true)
        }
    }

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
        bridge.computeStrokeLinks()
    }

    private fun notifyStrokeView() {
        Log.d("EinkDraw", "notifyStrokeView commitedStrokes=${buf.strokes.size} bufSize=${buf.size}")
        strokeView?.update(
            buf.strokes, currentTransform(), _bindGroups, bindHandler.currentPoints(),
            ocrResults.toList(), annotationMode || isBindMode,
            groupLabelOffsets.toMap(), clusterLabelOffsets.toMap(),
            selectedLabel,
            groupStrokeOffsets.toMap(),
            clusterStrokeOffsets.toMap(),
        )
    }

    private fun notifyStrokeViewLive() {
        val all = buf.allStrokes()
        strokeView?.update(all, currentTransform())
    }

    /**
     * The only place [isBindMode] / [isSelectMode] should be mutated externally.
     * Pushes the new mode to the SDK via [applyDrawingState] so drawing state and UI
     * mode can never diverge.
     */
    fun setMode(mode: ToolMode) {
        val wasBind = isBindMode
        val wasSelect = isSelectMode
        val modeChanged = (wasBind != (mode == ToolMode.TAG)) || (wasSelect != (mode == ToolMode.MOVE))
        isBindMode = (mode == ToolMode.TAG)
        isSelectMode = (mode == ToolMode.MOVE)
        if (wasBind && !isBindMode) {
            strokeView?.setBindPath(null)
        }
        if (wasSelect && !isSelectMode) {
            dragLabel = null; dragGroupId = null; dragClusterIdx = null
            selectedLabel = null
            onGroupSelectionChanged?.invoke(null)
        }
        // Reset the render layer on mode transitions to clear any stale finger-drawn
        // bind-mode lasso path or palm-touch artifacts.
        applyDrawingState(reset = modeChanged)
        notifyStrokeView()
    }

    // Back-compat wrappers kept for existing tests; new code should use setMode().
    fun enterBindMode() = setMode(ToolMode.TAG)
    fun exitBindMode() = setMode(ToolMode.DRAW)
    fun enterSelectMode() = setMode(ToolMode.MOVE)
    fun exitSelectMode() = setMode(ToolMode.DRAW)

    private fun eraseBindGroupsNearPath(screenPath: List<Pair<Float, Float>>, transform: ViewTransform) {
        val docPath = screenPath.map { (x, y) -> transform.screenToDocX(x) to transform.screenToDocY(y) }
        val docR = ERASER_BIND_RADIUS / transform.scale
        fun eraserCrossesSegment(ax: Float, ay: Float, bx: Float, by: Float): Boolean =
            docPath.any { (px, py) -> pointToSegmentDist(px, py, ax, ay, bx, by) <= docR }
        val hitGroups = _bindGroups.filter { g ->
            val strokes = g.strokeIndices.mapNotNull { buf.strokes.getOrNull(it) }
            val sBounds = strokesBBox(strokes)
            if (sBounds != null && g.elementDocCenters.isNotEmpty()) {
                val (scx, scy) = sBounds
                g.elementDocCenters.any { (ecx, ecy) -> eraserCrossesSegment(scx, scy, ecx, ecy) }
            } else false
        }
        if (hitGroups.isNotEmpty()) {
            hitGroups.forEach { removeBindGroup(it.id) }
            notifyStrokeView()
            onBindGroupsChanged?.invoke()
        }
    }

    private fun strokesBBox(strokes: List<Stroke>): Pair<Float, Float>? {
        var minX = Float.MAX_VALUE; var maxX = -Float.MAX_VALUE
        var minY = Float.MAX_VALUE; var maxY = -Float.MAX_VALUE
        for (s in strokes) for ((x, y) in s.points) {
            if (x < minX) minX = x; if (x > maxX) maxX = x
            if (y < minY) minY = y; if (y > maxY) maxY = y
        }
        return if (minX < maxX) (minX + maxX) / 2f to (minY + maxY) / 2f else null
    }

    private fun commitClusterOffset(clusterIdx: Int) {
        val (dx, dy) = clusterStrokeOffsets.remove(clusterIdx) ?: return
        clusterLabelOffsets.remove(clusterIdx)
        if (dx == 0f && dy == 0f) return
        val result = ocrResults.getOrNull(clusterIdx) ?: return
        buf.shiftStrokes(result.strokeIndices, dx, dy)
        ocrResults[clusterIdx] = result.copy(
            docX = result.docX + dx, docY = result.docY + dy, minDocY = result.minDocY + dy,
        )
        notifyStrokeView()
    }

    private fun commitGroupOffset(groupId: Int) {
        val (dx, dy) = groupStrokeOffsets.remove(groupId) ?: return
        groupLabelOffsets.remove(groupId)
        if (dx == 0f && dy == 0f) return
        val idx = _bindGroups.indexOfFirst { it.id == groupId }
        if (idx >= 0) {
            val group = _bindGroups[idx]
            buf.shiftStrokes(group.strokeIndices, dx, dy)
            _bindGroups[idx] = group.copy(
                markerDocX = group.markerDocX + dx,
                markerDocY = group.markerDocY + dy,
                strokeDocCenters = group.strokeDocCenters.map { (x, y) -> x + dx to y + dy },
            )
            tryMergeNearbyGroups(groupId)
        }
        notifyStrokeView()
        syncBindGroupsToWebView()
        onBindGroupsChanged?.invoke()
    }

    private fun findMergeTarget(cx: Float, cy: Float): BindGroup? {
        for (group in _bindGroups) {
            val gc = group.strokeIndices.mapNotNull { idx ->
                buf.strokes.getOrNull(idx)?.let { strokeCentroid(it) }
            }
            if (gc.isEmpty()) continue
            val gcx = gc.map { it.first }.average().toFloat()
            val gcy = gc.map { it.second }.average().toFloat()
            val dx = gcx - cx; val dy = gcy - cy
            if (dx * dx + dy * dy <= MERGE_THRESHOLD_DOC * MERGE_THRESHOLD_DOC) return group
        }
        return null
    }

    private fun tryAbsorbStrokeIntoGroup(strokeIdx: Int) {
        val stroke = buf.strokes.getOrNull(strokeIdx) ?: return
        if (stroke.points.size < 3) return
        val (scx, scy) = strokeCentroid(stroke)
        val target = findMergeTarget(scx, scy) ?: return
        val idx = _bindGroups.indexOfFirst { it.id == target.id }
        if (idx < 0) return
        val merged = target.strokeIndices + strokeIdx
        val allCentroids = merged.mapNotNull { i -> buf.strokes.getOrNull(i)?.let { strokeCentroid(it) } }
        val allC = allCentroids + target.elementDocCenters
        _bindGroups[idx] = target.copy(
            strokeIndices = merged,
            markerDocX = allC.map { it.first }.average().toFloat(),
            markerDocY = allC.map { it.second }.average().toFloat(),
            strokeDocCenters = allCentroids,
            recognizedText = null,
        )
        ocrResults.removeAll { r -> strokeIdx in r.strokeIndices }
        ocrManager?.cancel()
        val affectedStrokes = merged.mapNotNull { buf.strokes.getOrNull(it) }
        ocrManager?.evictCacheFor(affectedStrokes)
        onBindGroupsChanged?.invoke()
        syncBindGroupsToWebView()
    }

    private fun tryMergeNearbyGroups(movedGroupId: Int) {
        val moved = _bindGroups.firstOrNull { it.id == movedGroupId } ?: return
        val movedCentroids = moved.strokeIndices.mapNotNull { buf.strokes.getOrNull(it)?.let { s -> strokeCentroid(s) } }
        if (movedCentroids.isEmpty()) return
        val mcx = movedCentroids.map { it.first }.average().toFloat()
        val mcy = movedCentroids.map { it.second }.average().toFloat()
        val neighbor = _bindGroups.firstOrNull { g ->
            if (g.id == movedGroupId) return@firstOrNull false
            val gc = g.strokeIndices.mapNotNull { buf.strokes.getOrNull(it)?.let { s -> strokeCentroid(s) } }
            if (gc.isEmpty()) return@firstOrNull false
            val gcx = gc.map { it.first }.average().toFloat()
            val gcy = gc.map { it.second }.average().toFloat()
            val dx = gcx - mcx; val dy = gcy - mcy
            dx * dx + dy * dy <= MERGE_THRESHOLD_DOC * MERGE_THRESHOLD_DOC
        } ?: return
        // Merge moved into neighbor
        val idx = _bindGroups.indexOfFirst { it.id == neighbor.id }
        if (idx < 0) return
        val merged = neighbor.strokeIndices + moved.strokeIndices
        val mergedRefs = (neighbor.elementRefs + moved.elementRefs).distinctBy { Triple(it.sectionId, it.tag, it.text) }
        val mergedElemIdx = (neighbor.elementIndices + moved.elementIndices).distinct()
        val mergedElemCenters = (neighbor.elementDocCenters + moved.elementDocCenters).distinct()
        val allCentroids = merged.mapNotNull { i -> buf.strokes.getOrNull(i)?.let { strokeCentroid(it) } }
        val allC = allCentroids + mergedElemCenters
        _bindGroups[idx] = neighbor.copy(
            strokeIndices = merged,
            elementIndices = mergedElemIdx,
            elementRefs = mergedRefs,
            elementDocCenters = mergedElemCenters,
            markerDocX = allC.map { it.first }.average().toFloat(),
            markerDocY = allC.map { it.second }.average().toFloat(),
            strokeDocCenters = allCentroids,
            recognizedText = null,
        )
        _bindGroups.removeAll { it.id == movedGroupId }
        undoStack.removeAll { it is UndoAction.BindGroupAdded && it.groupId == movedGroupId }
        groupLabelOffsets.remove(movedGroupId)
        groupStrokeOffsets.remove(movedGroupId)
        ocrManager?.cancel()
        val affectedStrokes = merged.mapNotNull { buf.strokes.getOrNull(it) }
        ocrManager?.evictCacheFor(affectedStrokes)
        scheduleOcr()
    }

    fun scheduleReOcr(groupId: Int) {
        val idx = _bindGroups.indexOfFirst { it.id == groupId }
        if (idx < 0) return
        _bindGroups[idx] = _bindGroups[idx].copy(recognizedText = null)
        notifyStrokeView()
        scheduleOcr()
    }

    private val bindHandler: BindGestureHandler by lazy {
        BindGestureHandler(
            buf = buf,
            getTransform = ::currentTransform,
            elementLookup = bridge.asElementLookup(),
            onGestureComplete = { result -> completeBindGestureResult(result) },
            onTap = { screenX, screenY -> flashMarkerNear(screenX, screenY) },
        )
    }

    private fun completeBindGestureResult(result: BindGestureResult) {
        val docXs = result.docPolygon.map { it.first }
        val docYs = result.docPolygon.map { it.second }
        val markerX = docXs.average().toFloat()
        val markerY = docYs.average().toFloat()
        if (result.strokeIndices.isNotEmpty() || result.elements.isNotEmpty()) {
            val color = nextGroupColor()
            val centers = result.strokeIndices.mapNotNull { idx ->
                buf.strokes.getOrNull(idx)?.let { strokeCentroid(it) }
            }
            val elementCenters = result.elements.map { it.cx to it.cy }
            val allCenters = centers + elementCenters
            val midX = if (allCenters.isNotEmpty()) allCenters.map { it.first }.average().toFloat() else markerX
            val midY = if (allCenters.isNotEmpty()) allCenters.map { it.second }.average().toFloat() else markerY
            val groupId = nextGroupId++
            _bindGroups.add(BindGroup(
                id = groupId,
                color = color,
                strokeIndices = result.strokeIndices,
                elementIndices = result.elements.map { it.i },
                elementRefs = result.elements.map { ElementRef(it.section ?: it.id, it.tag, it.text) },
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
        bridge.flashGroup(group.elementIndices, group.color)
    }

    fun removeBindGroup(groupId: Int) {
        _bindGroups.removeAll { it.id == groupId }
        undoStack.removeAll { it is UndoAction.BindGroupAdded && it.groupId == groupId }
        groupLabelOffsets.remove(groupId)
        groupStrokeOffsets.remove(groupId)
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
    private var nextGroupId = 0
    var onBindComplete: (() -> Unit)? = null
    var onBindGroupsChanged: (() -> Unit)? = null
    var onOcrResultsChanged: ((List<OcrResult>) -> Unit)? = null
    /** Called with the selected group id when a group is tapped in select mode, null when deselected. */
    var onGroupSelectionChanged: ((groupId: Int?) -> Unit)? = null
    /** Called when the user taps empty space in select mode — host should exit the mode. */
    var onSelectModeExitRequested: (() -> Unit)? = null

    // Select / move mode
    private var isSelectMode = false
    private val groupLabelOffsets = mutableMapOf<Int, Pair<Float, Float>>()
    private val groupStrokeOffsets = mutableMapOf<Int, Pair<Float, Float>>()
    private val clusterLabelOffsets = mutableMapOf<Int, Pair<Float, Float>>()
    private val clusterStrokeOffsets = mutableMapOf<Int, Pair<Float, Float>>()
    private var selectedLabel: LabelId? = null
    private var dragLabel: LabelId? = null
    private var dragGroupId: Int? = null
    private var dragClusterIdx: Int? = null
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
        bridge.queryElementMap(callback)
    }

    fun clearBindGroups() {
        _bindGroups.clear()
        undoStack.removeAll { it is UndoAction.BindGroupAdded }
        onBindGroupsChanged?.invoke()
        notifyStrokeView()
        syncBindGroupsToWebView()
    }

    fun refreshStrokeLinks() {
        bridge.computeStrokeLinks()
    }

    private fun syncBindGroupsToWebView() {
        bridge.applyBindGroups(_bindGroups)
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
        private const val SCROLL_BLOCK_MS = 500L
        private const val MERGE_THRESHOLD_DOC = 200f
        private const val ERASER_BIND_RADIUS = 40f
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
