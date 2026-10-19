package com.flakm.einkbridge

import android.annotation.SuppressLint
import android.app.AlertDialog
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.pm.PackageManager
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import android.view.MotionEvent
import android.view.View
import android.webkit.*
import android.widget.*
import androidx.activity.OnBackPressedCallback
import androidx.appcompat.app.AppCompatActivity
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.lifecycleScope
import androidx.recyclerview.widget.LinearLayoutManager
import androidx.recyclerview.widget.RecyclerView
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.collect
import okhttp3.OkHttpClient
import okhttp3.Request
import org.json.JSONObject
import com.onyx.android.sdk.api.device.epd.EpdController
import com.onyx.android.sdk.api.device.epd.UpdateMode
import org.json.JSONTokener
import java.util.concurrent.TimeUnit

private const val TAG = "EinkMain"
private const val INACTIVITY_TIMEOUT_MS = 5 * 60 * 1000L
private const val NOTIFICATION_ID_UPDATE = 1001
// Channel importance is immutable on Android; use a new ID to quiet existing installs too.
internal const val NOTIFICATION_CHANNEL_ID = "eink_updates_quiet"
/** The stroke sizes the toolbar offers, in the pen's units. */
internal val STROKE_SIZES = floatArrayOf(2f, 4f, 8f, 14f)

class MainActivity : AppCompatActivity() {
    internal lateinit var webView: WebView
    internal lateinit var sessionListContainer: View
    internal lateinit var penToolbar: View
    internal lateinit var sessionList: RecyclerView
    private lateinit var serverInput: EditText
    private lateinit var metricsInput: EditText
    private lateinit var serverUrlText: TextView
    internal lateinit var emptyState: TextView
    private lateinit var strokeView: StrokeView
    internal lateinit var adapter: SessionAdapter
    private lateinit var sessionRepo: SessionRepository
    private lateinit var submissionManager: SubmissionManager
    private var penOverlay: PenOverlay? = null
    private lateinit var docCache: DocumentCache
    private var wsClient: WebSocketClient? = null
    private var loadedVersion: Int = 0
    private var savedScrollY: Int = 0
    private lateinit var ocrOverlay: View
    private lateinit var ocrStatus: TextView
    private val client = HttpClients.withToken(
        OkHttpClient.Builder()
            .connectTimeout(5, TimeUnit.SECONDS)
            .readTimeout(10, TimeUnit.SECONDS)
            .build()
    ) { sessionRepo.token() }

    private val scope = CoroutineScope(Dispatchers.Main + SupervisorJob())
    private lateinit var sessionViewModel: SessionViewModel
    private var serverUrl = ""
    private var pushgatewayUrl = ""
    private var currentSessionId: String? = null
    private var inactivityJob: Job? = null
    private var metricsJob: Job? = null
    private var wsInSleep = false
    private var activityResumed = false
    private val hideRoutineStatus = Runnable {
        findViewById<TextView>(R.id.routineStatus).visibility = View.GONE
    }
    @Volatile private var annotationSentAt: Long = 0L
    private lateinit var drawControls: android.widget.LinearLayout
    private lateinit var contextBar: android.widget.LinearLayout
    private var currentContextGroupId: Int? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        createNotificationChannel()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf(android.Manifest.permission.POST_NOTIFICATIONS), NOTIFICATION_ID_UPDATE)
        }
        WebView.setWebContentsDebuggingEnabled(true)
        sessionRepo = SessionRepository(this)
        submissionManager = SubmissionManager(client)
        docCache = DocumentCache(this)
        sessionViewModel = ViewModelProvider(this)[SessionViewModel::class.java]
        serverUrl = sessionViewModel.serverUrl
        pushgatewayUrl = sessionRepo.pushgatewayUrl()

        webView = findViewById(R.id.webView)
        sessionListContainer = findViewById(R.id.sessionListContainer)
        penToolbar = findViewById(R.id.penToolbar)
        sessionList = findViewById(R.id.sessionList)
        serverInput = findViewById(R.id.serverInput)
        metricsInput = findViewById(R.id.metricsInput)
        serverUrlText = findViewById(R.id.serverUrl)
        emptyState = findViewById(R.id.emptyState)
        strokeView = findViewById(R.id.strokeView)
        ocrOverlay = findViewById(R.id.processingOverlay)
        ocrStatus = findViewById(R.id.processingStatus)
        findViewById<Button>(R.id.btnCancelProcessing).setOnClickListener { cancelProcessing() }

        serverInput.setText(serverUrl)
        findViewById<EditText>(R.id.tokenInput).setText(sessionRepo.token())
        metricsInput.setText(pushgatewayUrl)
        serverUrlText.text = if (serverUrl.isNotEmpty()) "Connected: $serverUrl" else "Not connected"

        adapter = SessionAdapter(
            onClick = { session -> openSession(session.id) },
            onStarToggle = { session ->
                lifecycleScope.launch {
                    val ok = sessionViewModel.toggleStar(session.id, !session.starred)
                    val msg = if (ok) {
                        if (!session.starred) "Starred" else "Unstarred"
                    } else "Star toggle failed"
                    Toast.makeText(this@MainActivity, msg, Toast.LENGTH_SHORT).show()
                }
            },
        )
        sessionList.layoutManager = LinearLayoutManager(this)
        sessionList.adapter = adapter

        setupWebView()
        setupPenToolbar()

        onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
            override fun handleOnBackPressed() {
                if (webView.visibility == View.VISIBLE) {
                    showSessionList()
                    startPolling()
                } else {
                    isEnabled = false
                    onBackPressedDispatcher.onBackPressed()
                }
            }
        })

        lifecycleScope.launch {
            sessionViewModel.state.collect { listState ->
                val hadSessions = adapter.itemCount
                adapter.setPendingStrokes(listState.pendingStrokes)
                adapter.setCachedSessions(listState.cachedSessions)
                adapter.submitList(listState.sessions)
                if (listState.sessions.isEmpty()) {
                    sessionList.visibility = View.GONE
                    emptyState.visibility = View.VISIBLE
                } else {
                    sessionList.visibility = View.VISIBLE
                    emptyState.visibility = View.GONE
                }
                if (listState.sessions.size > hadSessions && hadSessions > 0) {
                    showRoutineStatus("New review")
                }
                when (listState.connectionStatus) {
                    is ConnectionStatus.Online -> {
                        serverUrlText.text = "Connected: ${sessionViewModel.serverUrl}"
                        serverUrlText.setTextColor(Color.parseColor("#006600"))
                    }
                    is ConnectionStatus.Offline -> {
                        serverUrlText.text = "Offline \u2014 showing cached: ${sessionViewModel.serverUrl}"
                        serverUrlText.setTextColor(Color.parseColor("#FF9800"))
                    }
                    is ConnectionStatus.Unreachable -> {
                        serverUrlText.text = "Unreachable: ${sessionViewModel.serverUrl}"
                        serverUrlText.setTextColor(Color.parseColor("#CC0000"))
                    }
                    is ConnectionStatus.Unknown -> {}
                }
            }
        }

        findViewById<Button>(R.id.connectBtn).setOnClickListener {
            val url = serverInput.text.toString().trimEnd('/')
            sessionRepo.saveToken(findViewById<EditText>(R.id.tokenInput).text.toString())
            if (url.isNotEmpty()) {
                serverUrl = url
                sessionViewModel.setServerUrl(url)
                serverUrlText.text = "Connecting..."
                serverUrlText.setTextColor(Color.GRAY)
                scope.launch {
                    val reachable = sessionViewModel.checkReachable(url)
                    if (reachable) {
                        startPolling()
                    } else {
                        serverUrlText.text = "Unreachable: $serverUrl"
                        serverUrlText.setTextColor(Color.parseColor("#CC0000"))
                    }
                }
            }
        }

        if (sessionViewModel.serverUrl.isNotEmpty()) {
            sessionViewModel.startPolling()
        }

        findViewById<Button>(R.id.metricsBtn).setOnClickListener {
            pushgatewayUrl = metricsInput.text.toString().trimEnd('/')
            sessionRepo.savePushgatewayUrl(pushgatewayUrl)
        }

        findViewById<Button>(R.id.btnCleanNotes).setOnClickListener {
            AlertDialog.Builder(this)
                .setTitle("Clean all notes?")
                .setMessage("Remove every note from the server, pinned ones included. This deletes their files too.")
                .setPositiveButton("Clean") { _, _ ->
                    scope.launch {
                        val ok = sessionViewModel.cleanNotes()
                        showRoutineStatus(if (ok) "Notes cleaned" else "Could not clean the notes")
                    }
                }
                .setNegativeButton("Cancel", null)
                .show()
        }
        findViewById<Button>(R.id.btnClearSessions).setOnClickListener {
            AlertDialog.Builder(this)
                .setTitle("Clear finished sessions?")
                .setMessage("Remove all submitted, cancelled, and expired sessions from the server.")
                .setPositiveButton("Clear") { _, _ -> clearFinishedSessions() }
                .setNegativeButton("Cancel", null)
                .show()
        }
    }

    @SuppressLint("SetJavaScriptEnabled")
    private fun setupWebView() {
        webView.settings.javaScriptEnabled = true
        webView.settings.domStorageEnabled = true
        webView.settings.builtInZoomControls = true
        webView.settings.displayZoomControls = false
        webView.settings.useWideViewPort = true
        webView.settings.loadWithOverviewMode = false
        webView.webViewClient = object : android.webkit.WebViewClient() {
            override fun onPageStarted(view: WebView, url: String, favicon: android.graphics.Bitmap?) {
                penOverlay?.setInputEnabled(false)
                findViewById<Button>(R.id.btnRequestUpdate).isEnabled = false
            }
            override fun onPageFinished(view: WebView, url: String) {
                val sid = currentSessionId ?: return
                Log.d(TAG, "webview onPageFinished url=$url session=$sid")
                // The page is here, so the pen, the ink and the UPDATE button come back now,
                // before any of the polish below: the element map and the relocation only
                // adjust ink, and every one of their exits must leave the page drawable.
                penOverlay?.onPageReady()
                findViewById<Button>(R.id.btnRequestUpdate).isEnabled = true
                // The ink mapping comes from the page, so it has to be read as soon as the
                // page can answer -- otherwise the first strokes are drawn through the
                // fallback until something happens to scroll or resize.
                penOverlay?.refreshViewport()
                view.evaluateJavascript("Number((document.querySelector('meta[name=eink-version]') || {}).content || 0)") { version ->
                    if (currentSessionId == sid) {
                        loadedVersion = version?.toIntOrNull() ?: 0
                        reconcileAnnotations(sid)
                    }
                }
                val scrollY = savedScrollY
                if (scrollY > 0) {
                    savedScrollY = 0
                    view.post { view.scrollTo(0, scrollY) }
                }
                view.evaluateJavascript("document.documentElement.outerHTML") { html ->
                    if (html.isNullOrBlank() || html == "null") return@evaluateJavascript
                    val cleaned = try {
                        JSONTokener(html).nextValue() as? String
                    } catch (_: Exception) { null }
                    if (!cleaned.isNullOrBlank()) {
                        scope.launch(Dispatchers.IO) { docCache.saveHtml(sid, cleaned) }
                    }
                }
            }
            override fun onReceivedError(
                view: WebView, request: android.webkit.WebResourceRequest,
                error: android.webkit.WebResourceError
            ) {
                if (request.isForMainFrame) {
                    val sid = currentSessionId ?: return
                    Log.w(TAG, "webview onReceivedError url=${request.url} error=${error.description} session=$sid — loading cache")
                    val cached = docCache.loadHtml(sid)
                    if (cached != null) {
                        view.loadDataWithBaseURL(serverUrl, cached, "text/html", "utf-8", null)
                    } else {
                        Log.w(TAG, "webview onReceivedError: no cache available for session=$sid")
                    }
                }
            }
        }
    }

    private val styleButtons = mutableListOf<ImageButton>()
    internal lateinit var btnSize: ImageButton
    internal lateinit var sizeDrawer: LinearLayout
    private lateinit var btnLink: ImageButton
    private lateinit var btnColor: ImageButton
    internal lateinit var colorDrawer: LinearLayout
    private val colorDots = mutableListOf<View>()
    private val sizeDots = mutableListOf<View>()
    private lateinit var btnAnnotations: ImageButton
    private lateinit var btnSelect: ImageButton
    private var selectedStyleIndex = 0
    internal var toolMode: ToolMode = ToolMode.DRAW
        private set
    internal val bindModeActive: Boolean get() = toolMode == ToolMode.TAG
    internal val selectModeActive: Boolean get() = toolMode == ToolMode.MOVE
    private var annotationModeActive = false
    private var currentStrokeColor = Color.BLACK
    internal var currentStrokeWidth = STROKE_SIZES[1]
        private set
    private val epdHandler = Handler(Looper.getMainLooper())
    private var epdRestoreRunnable: Runnable? = null
    private var processingJob: Job? = null


    private val colorPalette = intArrayOf(
        Color.BLACK,
        Color.parseColor("#CC0000"),
        Color.parseColor("#2196F3"),
        Color.parseColor("#4CAF50"),
        Color.parseColor("#FF9800"),
        Color.parseColor("#9C27B0"),
        Color.parseColor("#009688"),
        Color.parseColor("#607D8B"),
    )

    @SuppressLint("ClickableViewAccessibility")
    private fun setupPenToolbar() {
        drawControls = findViewById(R.id.drawControls)
        contextBar = findViewById(R.id.contextBar)
        btnSize = findViewById(R.id.btnSize)
        sizeDrawer = findViewById(R.id.sizeDrawer)
        buildSizeDrawer()
        btnSize.setOnClickListener { togglePopover(sizeDrawer, btnSize) }
        val btnPencil = findViewById<ImageButton>(R.id.btnPencil)
        val btnBrush = findViewById<ImageButton>(R.id.btnBrush)
        val btnUndo = findViewById<ImageButton>(R.id.btnUndo)
        val btnClear = findViewById<ImageButton>(R.id.btnClear)
        btnLink = findViewById(R.id.btnLink)
        btnColor = findViewById(R.id.btnColor)
        colorDrawer = findViewById(R.id.colorDrawer)
        buildColorDrawer()
        btnColor.imageTintList = android.content.res.ColorStateList.valueOf(currentStrokeColor)
        btnColor.setOnClickListener { togglePopover(colorDrawer, btnColor) }
        btnAnnotations = findViewById(R.id.btnAnnotations)
        btnSelect = findViewById(R.id.btnSelect)

        val btnEraser = findViewById<ImageButton>(R.id.btnEraser)
        styleButtons.addAll(listOf(btnPencil, btnBrush, btnEraser))

        btnPencil.setOnClickListener {
            setToolMode(ToolMode.DRAW)
            selectStyle(0)
            penOverlay?.setStylePencil()
        }
        btnBrush.setOnClickListener {
            setToolMode(ToolMode.DRAW)
            selectStyle(1)
            penOverlay?.setStyleBrush()
        }
        btnEraser.setOnClickListener {
            setToolMode(ToolMode.DRAW)
            selectStyle(2)
            penOverlay?.setStyleEraser()
        }
        btnSelect.setOnClickListener {
            setToolMode(if (toolMode == ToolMode.MOVE) ToolMode.DRAW else ToolMode.MOVE)
        }
        btnUndo.setOnClickListener { penOverlay?.undoLastStroke() }
        btnClear.setOnClickListener {
            AlertDialog.Builder(this)
                .setTitle("Clear all?")
                .setMessage("This will remove all strokes and links.")
                .setPositiveButton("Clear") { _, _ ->
                    penOverlay?.clearStrokes()
                    penOverlay?.clearBindGroups()
                }
                .setNegativeButton("Cancel", null)
                .show()
        }
        btnLink.setOnClickListener {
            setToolMode(if (toolMode == ToolMode.TAG) ToolMode.DRAW else ToolMode.TAG)
        }
        btnAnnotations.setOnClickListener {
            annotationModeActive = !annotationModeActive
            btnAnnotations.isSelected = annotationModeActive
            penOverlay?.annotationMode = annotationModeActive
        }
        findViewById<ImageButton>(R.id.btnReOcr).setOnClickListener {
            val groupId = currentContextGroupId ?: return@setOnClickListener
            penOverlay?.scheduleReOcr(groupId)
        }
        findViewById<ImageButton>(R.id.btnRemoveGroup).setOnClickListener {
            val groupId = currentContextGroupId ?: return@setOnClickListener
            val group = penOverlay?.bindGroups?.find { it.id == groupId } ?: return@setOnClickListener
            penOverlay?.onDeleteBindGroup?.invoke(group)
        }
        findViewById<Button>(R.id.btnSubmit).setOnClickListener { submitAndGoBack() }
        findViewById<Button>(R.id.btnRequestUpdate).setOnClickListener { requestUpdate() }

        selectStyle(0)
    }

    /**
     * Mark a mode button as the active one.
     *
     * The button's own selected state is the signal -- a filled black pill against the
     * paper-white row. It used to be three signals at once (a background, an alpha and a
     * separate red dot) and it grew the glyph when selected, so the row moved under the
     * reader's finger.
     */
    private fun setModeButtonActive(btn: ImageButton, active: Boolean) {
        btn.isSelected = active
    }

    private fun selectStyle(idx: Int) {
        selectedStyleIndex = idx
        // The chosen style is inverted; the others stay at full contrast. Alpha made them
        // ghostly on e-ink and said "broken" rather than "not chosen".
        styleButtons.forEachIndexed { i, btn -> btn.isSelected = i == idx }
    }

    /**
     * Single source of truth for tool mode switching. All toolbar buttons route through here.
     * Updates UI indicators and pushes the new mode to [PenOverlay] atomically.
     */
    internal fun setToolMode(mode: ToolMode) {
        if (toolMode == mode) {
            // Even when the activity already thinks it is in this mode, the overlay is
            // told: the two can drift (a mode assigned directly during teardown, a gesture
            // the system interrupted), and a silent no-op is how a stuck mode becomes
            // permanent. Setting the same mode twice is free.
            penOverlay?.setMode(mode)
            return
        }
        toolMode = mode
        hidePopovers()
        setModeButtonActive(btnLink, mode == ToolMode.TAG)
        setModeButtonActive(btnSelect, mode == ToolMode.MOVE)
        val drawMode = mode == ToolMode.DRAW
        drawControls.visibility = if (drawMode) View.VISIBLE else View.GONE
        contextBar.visibility = View.GONE
        if (!drawMode) {
            currentContextGroupId = null
            styleButtons.forEach { it.isEnabled = false }
        } else {
            styleButtons.forEach { it.isEnabled = true }
            selectStyle(selectedStyleIndex)
        }
        penOverlay?.let { overlay ->
            overlay.onBindComplete = if (mode == ToolMode.TAG) {
                { setToolMode(ToolMode.DRAW) }
            } else null
            overlay.setMode(mode)
        }
    }

    internal fun enterBindMode() = setToolMode(ToolMode.TAG)
    internal fun exitBindMode() = setToolMode(ToolMode.DRAW)
    internal fun enterSelectMode() = setToolMode(ToolMode.MOVE)
    internal fun exitSelectMode() = setToolMode(ToolMode.DRAW)

    private fun buildColorDrawer() {
        colorDrawer.removeAllViews()
        colorDots.clear()
        val dm = resources.displayMetrics
        val dotPx = (32 * dm.density).toInt()
        val marginPx = (4 * dm.density).toInt()
        for (color in colorPalette) {
            val isSelected = color == currentStrokeColor
            val dot = View(this).apply {
                background = GradientDrawable().apply {
                    shape = GradientDrawable.OVAL
                    setColor(color)
                    setStroke(
                        if (isSelected) (4 * dm.density).toInt() else (1 * dm.density).toInt(),
                        if (isSelected) Color.BLACK else Color.argb(60, 0, 0, 0),
                    )
                }
                layoutParams = LinearLayout.LayoutParams(dotPx, dotPx).apply {
                    setMargins(marginPx, marginPx, marginPx, marginPx)
                }
                setOnClickListener {
                    currentStrokeColor = color
                    penOverlay?.setStrokeColor(color)
                    btnColor.imageTintList = android.content.res.ColorStateList.valueOf(color)
                    updateColorDotSelection()
                    hidePopovers()
                }
            }
            colorDrawer.addView(dot)
            colorDots.add(dot)
        }
    }

    /**
     * The size popover: one dot per size, drawn at that size, the chosen one ringed. It
     * replaced a 160dp slider that cost a fifth of the row for a value nobody set finely.
     */
    private fun buildSizeDrawer() {
        sizeDrawer.removeAllViews()
        sizeDots.clear()
        val dm = resources.displayMetrics
        val cellPx = (36 * dm.density).toInt()
        for (size in STROKE_SIZES) {
            val dot = View(this).apply {
                layoutParams = LinearLayout.LayoutParams(cellPx, cellPx)
                contentDescription = getString(R.string.btnSize_desc) + " ${size.toInt()}"
                setOnClickListener {
                    currentStrokeWidth = size
                    penOverlay?.setStrokeWidth(size)
                    updateSizeDotSelection()
                    hidePopovers()
                }
            }
            sizeDrawer.addView(dot)
            sizeDots.add(dot)
        }
        updateSizeDotSelection()
    }

    private fun updateSizeDotSelection() {
        val dm = resources.displayMetrics
        for ((i, dot) in sizeDots.withIndex()) {
            val size = STROKE_SIZES[i]
            val isSelected = size == currentStrokeWidth
            val inner = GradientDrawable().apply {
                shape = GradientDrawable.OVAL
                setColor(Color.BLACK)
            }
            val ring = GradientDrawable().apply {
                shape = GradientDrawable.OVAL
                setColor(Color.WHITE)
                setStroke((2 * dm.density).toInt(), if (isSelected) Color.BLACK else Color.TRANSPARENT)
            }
            // The dot's diameter follows the stroke: 2..14 units shown as 8..32dp.
            val inset = ((36 - (4 + size * 2)) / 2 * dm.density).toInt()
            dot.background = android.graphics.drawable.LayerDrawable(arrayOf(ring, inner)).apply {
                setLayerInset(1, inset, inset, inset, inset)
            }
        }
    }

    /** Show [popover] above [anchor], or hide it if it is the one already open. */
    private fun togglePopover(popover: View, anchor: View) {
        val wasOpen = popover.visibility == View.VISIBLE
        hidePopovers()
        if (wasOpen) return
        popover.visibility = View.VISIBLE
        anchorAbove(popover, anchor)
    }

    private fun hidePopovers() {
        colorDrawer.visibility = View.GONE
        sizeDrawer.visibility = View.GONE
    }

    /**
     * Centre [popover] over [anchor], clamped to the screen. It is a child of the root
     * frame at the bottom-left, so its horizontal position is set from the anchor.
     */
    private fun anchorAbove(popover: View, anchor: View) {
        val root = popover.parent as View
        var anchorLeft = 0
        var v: View? = anchor
        while (v != null && v !== root) {
            anchorLeft += v.left
            v = v.parent as? View
        }
        popover.measure(
            View.MeasureSpec.makeMeasureSpec(root.width, View.MeasureSpec.AT_MOST),
            View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED),
        )
        val margin = resources.getDimensionPixelSize(R.dimen.popover_margin)
        val left = anchorLeft + anchor.width / 2 - popover.measuredWidth / 2
        val maxLeft = (root.width - popover.measuredWidth - margin).coerceAtLeast(margin)
        popover.translationX = left.coerceIn(margin, maxLeft).toFloat()
    }

    private fun updateColorDotSelection() {
        val dm = resources.displayMetrics
        for ((i, dot) in colorDots.withIndex()) {
            val c = colorPalette[i]
            val isSelected = c == currentStrokeColor
            (dot.background as? GradientDrawable)?.setStroke(
                if (isSelected) (4 * dm.density).toInt() else (1 * dm.density).toInt(),
                if (isSelected) Color.BLACK else Color.argb(60, 0, 0, 0),
            )
        }
    }

    private fun setEpdMode(mode: UpdateMode) {
        try { EpdController.setViewDefaultUpdateMode(webView, mode) }
        catch (_: Exception) {}
    }

    private fun scheduleEpdRestore(delayMs: Long = 500) {
        epdRestoreRunnable?.let { epdHandler.removeCallbacks(it) }
        epdRestoreRunnable = Runnable { setEpdMode(UpdateMode.GC) }
        epdHandler.postDelayed(epdRestoreRunnable!!, delayMs)
    }

    /**
     * The page's images and its own fetches cannot carry an Authorization header, so the
     * cookie is set before the WebView asks for anything. The header still goes on the
     * main frame; the cookie exists for the subresources.
     */
    private fun installSessionCookie() {
        val token = sessionRepo.token().trim()
        if (token.isEmpty()) return
        android.webkit.CookieManager.getInstance().setCookie(serverUrl, "eink_token=$token")
        android.webkit.CookieManager.getInstance().flush()
    }

    internal fun loadSessionPage(sessionId: String) {
        installSessionCookie()
        val token = sessionRepo.token().trim()
        if (token.isEmpty()) {
            webView.loadUrl("$serverUrl/session/$sessionId")
        } else {
            webView.loadUrl(
                "$serverUrl/session/$sessionId",
                mapOf("Authorization" to "Bearer $token"),
            )
        }
    }

    private fun openSession(sessionId: String) {
        Log.i(TAG, "openSession id=$sessionId serverUrl=$serverUrl")
        MetricsReporter.recordSessionView()
        currentSessionId = sessionId
        sessionViewModel.stopPolling()
        penOverlay?.destroy()
        penOverlay = null
        annotationModeActive = false
        btnAnnotations.isSelected = false
        currentContextGroupId = null
        contextBar.visibility = View.GONE
        // Force the UI back to draw mode. setToolMode is a no-op if already DRAW.
        toolMode = ToolMode.DRAW
        setModeButtonActive(btnLink, false)
        setModeButtonActive(btnSelect, false)
        drawControls.visibility = View.VISIBLE
        styleButtons.forEach { it.isEnabled = true }
        selectStyle(selectedStyleIndex)
        wsClient?.disconnect()
        loadedVersion = 0
        MetricsReporter.recordWsConnection()
        wsClient = WebSocketClient(
            serverUrl = serverUrl,
            sessionId = sessionId,
            token = sessionRepo.token(),
            onMessage = { type, json -> runOnUiThread {
                if (currentSessionId == sessionId) handleWsMessage(type, json)
            } },
            onClosed = { Log.w(TAG, "ws onClosed session=$sessionId") },
        ).also { it.connect() }
        sessionListContainer.visibility = View.GONE
        webView.visibility = View.VISIBLE
        strokeView.visibility = View.VISIBLE
        penToolbar.visibility = View.VISIBLE
        loadSessionPage(sessionId)
        val buf = StrokeBuffer()
        sessionRepo.loadStrokes(sessionId, buf)
        val ocrClient = OcrClient({ serverUrl }, { sessionRepo.token() })
        val overlay = PenOverlay(webView, penToolbar, strokeView, buf = buf,
            onStrokesChanged = { sessionRepo.saveStrokes(sessionId, buf) },
        )
        val ocrRecognize: suspend (List<Stroke>) -> String? = { strokes ->
            try {
                val result = ocrClient.recognize(strokes)
                MetricsReporter.recordOcr(result != null)
                result
            } catch (e: Exception) {
                MetricsReporter.recordOcr(false)
                throw e
            }
        }
        val ocrManager = OcrManager(
            recognize = ocrRecognize,
            scope = scope,
            onGroupRecognized = { groupId, text -> overlay.onGroupOcrResult(groupId, text) },
            onUnboundResults = { results -> overlay.onUnboundOcrResults(results) },
            onPendingChanged = { remaining, total -> runOnUiThread { updateOcrProgress(remaining, total) } },
            onError = { msg -> runOnUiThread { Toast.makeText(this@MainActivity, msg, Toast.LENGTH_SHORT).show() } },
        )
        overlay.ocrManager = ocrManager
        overlay.init()
        overlay.setStrokeColor(currentStrokeColor)
        overlay.setStrokeWidth(currentStrokeWidth)
        overlay.onBindGroupsChanged = { sessionRepo.saveBindGroups(sessionId, overlay.bindGroups) }
        overlay.onOcrResultsChanged = { results -> sessionRepo.saveOcrResults(sessionId, results) }
        overlay.onDeleteBindGroup = { group ->
            AlertDialog.Builder(this)
                .setTitle("Remove link?")
                .setMessage("Unlink ${group.strokeIndices.size} stroke(s) and ${group.elementIndices.size} element(s)?")
                .setPositiveButton("Remove") { _, _ ->
                    overlay.removeBindGroup(group.id)
                    sessionRepo.saveBindGroups(sessionId, overlay.bindGroups)
                }
                .setNegativeButton("Cancel", null)
                .show()
        }
        overlay.onGroupSelectionChanged = { groupId ->
            runOnUiThread {
                currentContextGroupId = groupId
                contextBar.visibility = if (groupId != null && selectModeActive) View.VISIBLE else View.GONE
            }
        }
        overlay.onSelectModeExitRequested = { runOnUiThread { if (selectModeActive) exitSelectMode() } }
        val savedBindGroups = sessionRepo.loadBindGroups(sessionId)
        if (savedBindGroups.isNotEmpty()) overlay.loadBindGroups(savedBindGroups)
        val savedOcrResults = sessionRepo.loadOcrResults(sessionId)
        if (savedOcrResults.isNotEmpty()) overlay.loadOcrResults(savedOcrResults)
        penOverlay = overlay
        setEpdMode(UpdateMode.GC)
        webView.setOnScrollChangeListener { _, _, _, _, _ ->
            setEpdMode(UpdateMode.REGAL)
            scheduleEpdRestore(800)
            // The page may have panned or zoomed, which changes the mapping the ink is
            // drawn through; the report is what keeps the writing over its text.
            overlay.refreshViewport()
        }
        // A rotation resizes the view without recreating the activity (configChanges), so
        // this is the only signal that the mapping has changed. Without it the ink keeps
        // the old scale and lands a third of a paragraph away from the text.
        strokeView.addOnLayoutChangeListener { _, left, top, right, bottom, oldLeft, oldTop, oldRight, oldBottom ->
            val resized = (right - left) != (oldRight - oldLeft) || (bottom - top) != (oldBottom - oldTop)
            if (resized || left != oldLeft || top != oldTop) {
                Log.i(TAG, "view resized: ${oldRight - oldLeft}x${oldBottom - oldTop} -> ${right - left}x${bottom - top}: refreshing the ink mapping and the pen state")
                // A resize cancels in-flight gestures, so the pen must be re-armed as well as
                // re-mapped -- otherwise a finger the rotation swallowed keeps drawing off.
                overlay.onViewResized()
                overlay.refreshViewport()
                overlay.setViewportAfterLayout()
            }
        }
        resetInactivityTimer()
        startPeriodicMetrics()
    }

    private fun updateOcrProgress(remaining: Int, total: Int) {
        if (remaining == 0) {
            ocrOverlay.visibility = View.GONE
        } else {
            ocrStatus.text = "OCR $remaining / $total"
            ocrOverlay.visibility = View.VISIBLE
        }
    }

    private fun startPeriodicMetrics() {
        metricsJob?.cancel()
        metricsJob = scope.launch {
            while (true) {
                delay(60_000L)
                MetricsReporter.push(pushgatewayUrl)
            }
        }
    }

    private fun showSessionList() {
        Log.i(TAG, "showSessionList: leaving session=$currentSessionId")
        inactivityJob?.cancel()
        inactivityJob = null
        metricsJob?.cancel()
        metricsJob = null
        wsInSleep = false
        currentContextGroupId = null
        hidePopovers()
        toolMode = ToolMode.DRAW
        setModeButtonActive(btnLink, false)
        setModeButtonActive(btnSelect, false)
        drawControls.visibility = View.VISIBLE
        contextBar.visibility = View.GONE
        styleButtons.forEach { it.isEnabled = true }
        selectStyle(selectedStyleIndex)
        penOverlay?.onPaused()
        penOverlay?.destroy()
        penOverlay = null
        wsClient?.disconnect()
        wsClient = null
        currentSessionId = null
        webView.visibility = View.GONE
        strokeView.visibility = View.GONE
        penToolbar.visibility = View.GONE
        sessionListContainer.visibility = View.VISIBLE
    }

    private fun submitAndGoBack() {
        val sessionId = currentSessionId ?: run {
            Log.w(TAG, "submitAndGoBack: no currentSessionId, ignoring")
            return
        }
        val overlay = penOverlay ?: run {
            Log.w(TAG, "submitAndGoBack: no penOverlay session=$sessionId, ignoring")
            return
        }
        Log.i(TAG, "submitAndGoBack: starting session=$sessionId strokes=${overlay.buf.strokes.size}")
        overlay.queryElementMap { elements ->
            processingJob?.cancel()
            processingJob = scope.launch {
                try {
                    ocrStatus.text = "Submitting..."
                    ocrOverlay.visibility = View.VISIBLE
                    val pngData = overlay.exportToPng()
                    val strokeJson = overlay.exportStrokeJson()
                    val annotationsJson = submissionManager.buildAnnotationsJson(
                        overlay.buf.strokes, overlay.bindGroups, elements
                    )
                    val result = withTimeout(OCR_OPERATION_TIMEOUT_MS) {
                        withContext(Dispatchers.IO) {
                            submissionManager.submit(serverUrl, sessionId, annotationsJson, pngData, strokeJson)
                        }
                    }
                    ocrOverlay.visibility = View.GONE
                    when {
                        result.isSuccessful -> {
                            MetricsReporter.recordSubmission(true)
                            scope.launch { MetricsReporter.push(pushgatewayUrl) }
                            sessionRepo.clearSession(sessionId)
                            showRoutineStatus("Review submitted")
                            showSessionList()
                            startPolling()
                        }
                        result.isConflict -> {
                            MetricsReporter.recordSubmission(false)
                            Toast.makeText(this@MainActivity, "Session already submitted or expired", Toast.LENGTH_LONG).show()
                            showSessionList()
                            startPolling()
                        }
                        result is SubmitResult.Http -> {
                            MetricsReporter.recordSubmission(false)
                            Toast.makeText(this@MainActivity, "Submit failed: ${result.code}", Toast.LENGTH_SHORT).show()
                        }
                        result is SubmitResult.Failure -> {
                            MetricsReporter.recordSubmission(false)
                            Toast.makeText(this@MainActivity, "Error: ${result.exception.message}", Toast.LENGTH_SHORT).show()
                        }
                    }
                } catch (e: TimeoutCancellationException) {
                    Log.w(TAG, "submitAndGoBack: timed out session=$sessionId")
                    ocrOverlay.visibility = View.GONE
                    Toast.makeText(this@MainActivity, "Submit timed out", Toast.LENGTH_SHORT).show()
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    Log.e(TAG, "submitAndGoBack: exception session=$sessionId", e)
                    ocrOverlay.visibility = View.GONE
                    Toast.makeText(this@MainActivity, "Error: ${e.message}", Toast.LENGTH_SHORT).show()
                }
            }
        }
    }

    private fun checkVersionAndReload(sessionId: String) {
        scope.launch {
            try {
                val serverVersion = withContext(Dispatchers.IO) {
                    val req = Request.Builder().url("$serverUrl/api/sessions/$sessionId/feedback").build()
                    // This poll runs repeatedly; an unclosed body leaks a connection each time.
                    client.newCall(req).execute().use { resp ->
                        if (!resp.isSuccessful) return@withContext -1
                        val body = resp.body?.string() ?: return@withContext -1
                        val snapshot = org.json.JSONObject(body)
                        withContext(Dispatchers.Main) {
                            if (currentSessionId == sessionId) {
                                penOverlay?.consumeStrokes(snapshot.optJSONObject("last_content_update")?.optJSONArray("consumed_strokes"))
                                penOverlay?.let { sessionRepo.saveStrokes(sessionId, it.buf) }
                            }
                        }
                        snapshot.optInt("version", -1)
                    }
                }
                if (serverVersion > loadedVersion) {
                    Log.i(TAG, "reconnect: server version=$serverVersion > loaded=$loadedVersion — reloading")
                    loadedVersion = serverVersion
                    loadSessionPage(sessionId)
                } else {
                    Log.d(TAG, "reconnect: version up to date ($serverVersion)")
                }
            } catch (_: Exception) {}
        }
    }

    private fun cancelProcessing() {
        processingJob?.cancel()
        processingJob = null
        annotationSentAt = 0L
        ocrOverlay.visibility = View.GONE
        showRoutineStatus("Cancelled")
    }

    private fun requestUpdate() {
        val sessionId = currentSessionId ?: run {
            Log.w(TAG, "requestUpdate: no currentSessionId — skipping")
            return
        }
        val overlay = penOverlay ?: run {
            Log.w(TAG, "requestUpdate: penOverlay is null — skipping")
            return
        }
        Log.i(TAG, "requestUpdate: querying element map session=$sessionId strokes=${overlay.buf.strokes.size}")
        val documentVersion = loadedVersion
        overlay.queryElementMap { elements ->
            if (currentSessionId != sessionId || loadedVersion != documentVersion) return@queryElementMap
            sessionRepo.saveElementMap(sessionId, elements)
            Log.i(TAG, "requestUpdate: callback elements=${elements.size} session=$sessionId")
            processingJob?.cancel()
            processingJob = scope.launch {
                try {
                    ocrStatus.text = "Sending annotations..."
                    ocrOverlay.visibility = View.VISIBLE
                    val annotationsJson = submissionManager.buildAnnotationsJson(
                        overlay.buf.strokes, overlay.bindGroups, elements
                    )
                    Log.i(TAG, "requestUpdate: sending to server session=$sessionId")
                    annotationSentAt = SystemClock.elapsedRealtime()
                    val ok = withTimeout(OCR_OPERATION_TIMEOUT_MS) {
                        withContext(Dispatchers.IO) {
                            submissionManager.requestUpdate(serverUrl, sessionId, annotationsJson, documentVersion.takeIf { it > 0 })
                        }
                    }
                    Log.i(TAG, "requestUpdate: server response ok=$ok session=$sessionId")
                    if (!ok) {
                        annotationSentAt = 0L
                        ocrOverlay.visibility = View.GONE
                        Toast.makeText(this@MainActivity, "Request failed", Toast.LENGTH_SHORT).show()
                    } else {
                        ocrOverlay.visibility = View.GONE
                        findViewById<Button>(R.id.btnRequestUpdate).text = "Waiting for agent…"
                    }
                } catch (e: TimeoutCancellationException) {
                    annotationSentAt = 0L
                    Log.w(TAG, "requestUpdate: timed out session=$sessionId")
                    ocrOverlay.visibility = View.GONE
                    Toast.makeText(this@MainActivity, "Request timed out", Toast.LENGTH_SHORT).show()
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    annotationSentAt = 0L
                    Log.e(TAG, "requestUpdate: exception session=$sessionId", e)
                    ocrOverlay.visibility = View.GONE
                    Toast.makeText(this@MainActivity, "Error: ${e.message}", Toast.LENGTH_SHORT).show()
                }
            }
        }
    }

    /**
     * Moves the ink to follow text that moved. This only adjusts ink: the pen, the ink view
     * and the UPDATE button were given back by `onPageFinished`, and nothing here may take
     * them away or be relied on to give them back -- this runs after a JS round-trip that may
     * never answer, and bails on any sign the page changed under it.
     */
    private fun reconcileAnnotations(sid: String) {
        val overlay = penOverlay ?: return
        val version = loadedVersion
        overlay.queryElementMap { current ->
            if (currentSessionId != sid) return@queryElementMap
            val previous = sessionRepo.loadElementMap(sid)
            when (reconcilePlan(previous, current, overlay.buf.isEmpty)) {
                ReconcilePlan.NOTHING -> return@queryElementMap
                ReconcilePlan.ADOPT_MAP -> {
                    sessionRepo.saveElementMap(sid, current)
                    return@queryElementMap
                }
                ReconcilePlan.RELOCATE -> {}
            }
            val original = overlay.buf.strokes
            val groups = overlay.bindGroups.toList()
            scope.launch {
                try {
                    val targets = withContext(Dispatchers.IO) {
                        submissionManager.reanchor(serverUrl, sid, previous, current)
                    }
                    if (currentSessionId != sid || loadedVersion != version) return@launch
                    if (overlay.buf.strokes != original) {
                        // The reader drew while the server was answering; the ink stays as drawn.
                        Log.i(TAG, "Annotation relocation skipped: ink changed during the round-trip session=$sid")
                        return@launch
                    }
                    val ink = relocateInk(original, groups, previous, current, targets)
                    Log.i(TAG, "Annotation relocation session=$sid version=$version retained=${ink.strokes.size} archived=${ink.orphanCount}")
                    sessionRepo.archiveInk(sid, overlay.buf, groups)
                    overlay.applyRelocation(ink)
                    sessionRepo.saveStrokes(sid, overlay.buf)
                    sessionRepo.saveElementMap(sid, current)
                    if (ink.orphanCount > 0) Toast.makeText(this@MainActivity,
                        "${ink.orphanCount} strokes archived: their text could not be located safely", Toast.LENGTH_LONG).show()
                } catch (e: Exception) {
                    // Keep the old layout for retry; never label stale ink as relocated.
                    Log.w(TAG, "Annotation relocation failed", e)
                    Toast.makeText(this@MainActivity, "Could not reposition annotations; reopen to retry", Toast.LENGTH_LONG).show()
                }
            }
        }
    }

    private fun clearFinishedSessions() {
        scope.launch {
            val ok = sessionViewModel.clearFinishedSessions()
            if (ok) {
                sessionViewModel.startPolling()
            } else {
                Toast.makeText(this@MainActivity, "Failed to clear sessions", Toast.LENGTH_SHORT).show()
            }
        }
    }

    private fun startPolling() { sessionViewModel.startPolling() }

    private fun resetInactivityTimer() {
        inactivityJob?.cancel()
        if (currentSessionId == null) return
        inactivityJob = scope.launch {
            delay(INACTIVITY_TIMEOUT_MS)
            Log.i(TAG, "inactivity timeout — disconnecting ws for deep sleep")
            scope.launch { MetricsReporter.push(pushgatewayUrl) }
            wsClient?.disconnect()
            wsInSleep = true
        }
    }

    override fun dispatchTouchEvent(event: MotionEvent?): Boolean {
        if (event?.action == MotionEvent.ACTION_DOWN && currentSessionId != null) {
            if (wsInSleep) {
                Log.i(TAG, "waking ws from inactivity sleep")
                wsInSleep = false
                wsClient?.connect()
            }
            resetInactivityTimer()
        }
        return super.dispatchTouchEvent(event)
    }

    override fun onResume() {
        super.onResume()
        activityResumed = true
        getSystemService(NotificationManager::class.java).cancel(NOTIFICATION_ID_UPDATE)
        if (currentSessionId != null) {
            penOverlay?.onResumed()
        }
    }

    override fun onPause() {
        activityResumed = false
        penOverlay?.onPaused()
        super.onPause()
    }

    override fun onDestroy() {
        findViewById<TextView>(R.id.routineStatus).removeCallbacks(hideRoutineStatus)
        inactivityJob?.cancel()
        penOverlay?.destroy()
        wsClient?.disconnect()
        scope.cancel()
        super.onDestroy()
    }

    private fun createNotificationChannel() {
        val channel = NotificationChannel(
            NOTIFICATION_CHANNEL_ID,
            "Document Updates",
            NotificationManager.IMPORTANCE_LOW,
        ).apply {
            setSound(null, null)
            enableVibration(false)
            setShowBadge(false)
        }
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(channel)
        // Remove the pre-existing loud channel so it stops appearing in system settings.
        manager.deleteNotificationChannel("eink_updates")
    }

    internal fun showRoutineStatus(message: String) {
        findViewById<TextView>(R.id.routineStatus).apply {
            removeCallbacks(hideRoutineStatus)
            text = message
            visibility = View.VISIBLE
            postDelayed(hideRoutineStatus, 4000)
        }
    }

    internal fun notifyDocumentUpdated() {
        // The inline label is sufficient while reading; no duplicate system alert.
        if (activityResumed) return
        val notification = Notification.Builder(this, NOTIFICATION_CHANNEL_ID)
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setContentTitle("Document updated")
            .setContentText("Ready to read")
            .setOnlyAlertOnce(true)
            .setAutoCancel(true)
            .build()
        getSystemService(NotificationManager::class.java)
            .notify(NOTIFICATION_ID_UPDATE, notification)
    }

    internal fun handleWsMessage(type: String, json: JSONObject) {
        Log.d(TAG, "handleWsMessage type=$type session=$currentSessionId json=${json.toString().take(200)}")
        runOnUiThread {
            when (type) {
                "version_updated" -> {
                    val version = json.optInt("version")
                    // The line and the bar described work that is now done. Reset them
                    // before the session check: a version can land after the reader has
                    // gone back to the list, and the button must not keep saying
                    // "Waiting for agent" there either.
                    findViewById<Button>(R.id.btnRequestUpdate).text = "Update"
                    findViewById<ProgressBar>(R.id.agentProgress).visibility = View.GONE
                    val sid = currentSessionId ?: return@runOnUiThread
                    if (version <= loadedVersion) return@runOnUiThread
                    val sentAt = annotationSentAt
                    if (sentAt != 0L) {
                        val roundTripMs = SystemClock.elapsedRealtime() - sentAt
                        annotationSentAt = 0L
                        MetricsReporter.recordUpdateRoundTrip(roundTripMs)
                        Log.i(TAG, "version_updated: round-trip=${roundTripMs}ms")
                        scope.launch { MetricsReporter.push(pushgatewayUrl) }
                    }
                    penOverlay?.also { overlay ->
                        overlay.consumeStrokes(json.optJSONArray("consumed_strokes"))
                        sessionRepo.saveStrokes(sid, overlay.buf)
                    }
                    Log.i(TAG, "version_updated: version=$version — reloading webview")
                    loadedVersion = version
                    ocrOverlay.visibility = View.GONE
                    savedScrollY = webView.scrollY
                    loadSessionPage(sid)
                    showRoutineStatus("Document updated")
                    notifyDocumentUpdated()
                }
                "session_submitted" -> {
                    Log.i(TAG, "session_submitted: going back to session list")
                    showRoutineStatus("Review submitted")
                    showSessionList()
                    startPolling()
                }
                "agent_progress" -> {
                    // Status, not prose: the agent says what it is doing while it works,
                    // in place of the static "Waiting for agent…".
                    val message = json.optString("message")
                    if (message.isNotBlank()) {
                        Log.i(TAG, "agent_progress: $message")
                        findViewById<Button>(R.id.btnRequestUpdate).text = message
                        findViewById<TextView>(R.id.processingStatus).text = message
                        ocrOverlay.visibility = View.VISIBLE
                    }
                    val bar = findViewById<ProgressBar>(R.id.agentProgress)
                    val step = json.optInt("step")
                    val of = json.optInt("of")
                    if (step > 0 && of > 0) {
                        bar.max = of
                        bar.progress = step
                        bar.visibility = View.VISIBLE
                    } else {
                        bar.visibility = View.GONE
                    }
                }
                "annotation_result" -> {
                    val version = json.optInt("version")
                    Log.i(TAG, "annotation_result: version=$version — waiting for document update")
                    ocrOverlay.visibility = View.GONE
                    findViewById<ProgressBar>(R.id.agentProgress).visibility = View.GONE
                    findViewById<Button>(R.id.btnRequestUpdate).text = "Waiting for agent…"
                    showRoutineStatus("Feedback saved · awaiting reply")
                }
                "error" -> {
                    val errMsg = json.optString("message", "Unknown error")
                    Log.w(TAG, "server error: $errMsg")
                    ocrOverlay.visibility = View.GONE
                    Toast.makeText(this, errMsg, Toast.LENGTH_LONG).show()
                }
                else -> Log.w(TAG, "unhandled ws message type=$type")
            }
        }
    }

}

data class SessionInfo(
    val id: String,
    val title: String,
    val status: String,
    val createdAt: String,
    val updatedAt: String,
    val starred: Boolean = false,
    val originCwd: String? = null,
)
