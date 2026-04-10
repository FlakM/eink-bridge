package com.flakm.einkbridge

import android.annotation.SuppressLint
import android.app.AlertDialog
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.pm.PackageManager
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.media.RingtoneManager
import android.os.Build
import android.os.Bundle
import android.os.SystemClock
import android.os.Vibrator
import android.os.VibrationEffect
import android.util.Log
import android.view.Gravity
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
import org.json.JSONTokener
import java.util.concurrent.TimeUnit

private const val TAG = "EinkMain"
private const val INACTIVITY_TIMEOUT_MS = 5 * 60 * 1000L
private const val NOTIFICATION_ID_UPDATE = 1001
private const val NOTIFICATION_CHANNEL_ID = "eink_updates"

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
    private val client = OkHttpClient.Builder()
        .connectTimeout(5, TimeUnit.SECONDS)
        .readTimeout(10, TimeUnit.SECONDS)
        .build()

    private val scope = CoroutineScope(Dispatchers.Main + SupervisorJob())
    private lateinit var sessionViewModel: SessionViewModel
    private var serverUrl = ""
    private var pushgatewayUrl = ""
    private var currentSessionId: String? = null
    private var inactivityJob: Job? = null
    private var wsInSleep = false
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

        serverInput.setText(serverUrl)
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
                    @Suppress("DEPRECATION")
                    (getSystemService(VIBRATOR_SERVICE) as? Vibrator)?.vibrate(200)
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
            override fun onPageFinished(view: WebView, url: String) {
                val sid = currentSessionId ?: return
                Log.d(TAG, "webview onPageFinished url=$url session=$sid")
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

    private val styleButtons = mutableListOf<Button>()
    internal lateinit var strokeSlider: SeekBar
    private lateinit var btnLink: Button
    private lateinit var btnColor: Button
    private lateinit var btnAnnotations: Button
    private lateinit var btnSelect: Button
    private var selectedStyleIndex = 0
    internal var toolMode: ToolMode = ToolMode.DRAW
        private set
    internal val bindModeActive: Boolean get() = toolMode == ToolMode.TAG
    internal val selectModeActive: Boolean get() = toolMode == ToolMode.MOVE
    private var annotationModeActive = false
    private var currentStrokeColor = Color.BLACK

    private val styleDotIds = listOf(R.id.dotPencil, R.id.dotBrush, R.id.dotEraser)

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
        strokeSlider = findViewById(R.id.strokeSlider)
        val btnPencil = findViewById<Button>(R.id.btnPencil)
        val btnBrush = findViewById<Button>(R.id.btnBrush)
        val btnUndo = findViewById<Button>(R.id.btnUndo)
        val btnClear = findViewById<Button>(R.id.btnClear)
        btnLink = findViewById(R.id.btnLink)
        btnColor = findViewById(R.id.btnColor)
        btnAnnotations = findViewById(R.id.btnAnnotations)
        btnSelect = findViewById(R.id.btnSelect)

        val btnEraser = findViewById<Button>(R.id.btnEraser)
        styleButtons.addAll(listOf(btnPencil, btnBrush, btnEraser))

        strokeSlider.setOnSeekBarChangeListener(object : SeekBar.OnSeekBarChangeListener {
            override fun onProgressChanged(seekBar: SeekBar, progress: Int, fromUser: Boolean) {
                penOverlay?.setStrokeWidth(progress.toFloat())
            }
            override fun onStartTrackingTouch(seekBar: SeekBar) {}
            override fun onStopTrackingTouch(seekBar: SeekBar) {}
        })

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
        btnColor.setOnClickListener { showColorPicker() }
        btnAnnotations.setOnClickListener {
            annotationModeActive = !annotationModeActive
            btnAnnotations.alpha = if (annotationModeActive) 1.0f else 0.35f
            btnAnnotations.textSize = if (annotationModeActive) 30f else 26f
            findViewById<View>(R.id.dotAnnotations).visibility =
                if (annotationModeActive) View.VISIBLE else View.GONE
            penOverlay?.annotationMode = annotationModeActive
        }
        findViewById<Button>(R.id.btnReOcr).setOnClickListener {
            val groupId = currentContextGroupId ?: return@setOnClickListener
            penOverlay?.scheduleReOcr(groupId)
        }
        findViewById<Button>(R.id.btnRemoveGroup).setOnClickListener {
            val groupId = currentContextGroupId ?: return@setOnClickListener
            val group = penOverlay?.bindGroups?.find { it.id == groupId } ?: return@setOnClickListener
            penOverlay?.onDeleteBindGroup?.invoke(group)
        }
        findViewById<Button>(R.id.btnSubmit).setOnClickListener { submitAndGoBack() }
        findViewById<Button>(R.id.btnRequestUpdate).setOnClickListener { requestUpdate() }

        selectStyle(0)
    }

    private fun setModeButtonActive(btn: Button, dotId: Int, active: Boolean) {
        if (active) {
            btn.background = android.graphics.drawable.GradientDrawable().apply {
                shape = android.graphics.drawable.GradientDrawable.RECTANGLE
                cornerRadius = resources.displayMetrics.density * 8
                setColor(Color.parseColor("#222222"))
            }
            btn.setTextColor(Color.WHITE)
            btn.alpha = 1.0f
        } else {
            btn.setBackgroundColor(Color.TRANSPARENT)
            btn.setTextColor(Color.BLACK)
            btn.alpha = 0.5f
        }
        btn.textSize = if (active) 36f else 32f
        findViewById<View>(dotId).visibility = if (active) View.VISIBLE else View.GONE
    }

    private fun selectStyle(idx: Int) {
        selectedStyleIndex = idx
        styleButtons.forEachIndexed { i, btn ->
            val active = i == idx
            btn.alpha = if (active) 1.0f else 0.35f
            btn.textSize = if (active) 42f else 34f
        }
        styleDotIds.forEachIndexed { i, id ->
            findViewById<View>(id).visibility = if (i == idx) View.VISIBLE else View.GONE
        }
    }

    /**
     * Single source of truth for tool mode switching. All toolbar buttons route through here.
     * Updates UI indicators and pushes the new mode to [PenOverlay] atomically.
     */
    internal fun setToolMode(mode: ToolMode) {
        if (toolMode == mode) return
        toolMode = mode
        setModeButtonActive(btnLink, R.id.dotLink, mode == ToolMode.TAG)
        setModeButtonActive(btnSelect, R.id.dotSelect, mode == ToolMode.MOVE)
        val drawMode = mode == ToolMode.DRAW
        drawControls.visibility = if (drawMode) View.VISIBLE else View.GONE
        contextBar.visibility = View.GONE
        if (!drawMode) {
            currentContextGroupId = null
            styleDotIds.forEach { findViewById<View>(it).visibility = View.GONE }
            styleButtons.forEach { it.isEnabled = false; it.alpha = 0.2f }
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

    private fun showColorPicker() {
        val dm = resources.displayMetrics
        val dotPx = (44 * dm.density).toInt()
        val marginPx = (8 * dm.density).toInt()
        val paddingPx = (20 * dm.density).toInt()

        val container = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(paddingPx, paddingPx, paddingPx, paddingPx)
        }
        val dialog = AlertDialog.Builder(this).setView(container).create()

        for (row in colorPalette.toList().chunked(4)) {
            val rowLayout = LinearLayout(this).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = Gravity.CENTER
            }
            for (color in row) {
                val isSelected = color == currentStrokeColor
                val dot = View(this).apply {
                    background = GradientDrawable().apply {
                        shape = GradientDrawable.OVAL
                        setColor(color)
                        setStroke(
                            if (isSelected) (5 * dm.density).toInt() else (2 * dm.density).toInt(),
                            if (isSelected) Color.BLACK else Color.argb(80, 0, 0, 0),
                        )
                    }
                    layoutParams = LinearLayout.LayoutParams(dotPx, dotPx).apply {
                        setMargins(marginPx, marginPx, marginPx, marginPx)
                    }
                    setOnClickListener {
                        currentStrokeColor = color
                        penOverlay?.setStrokeColor(color)
                        btnColor.setTextColor(color)
                        dialog.dismiss()
                    }
                }
                rowLayout.addView(dot)
            }
            container.addView(rowLayout)
        }

        dialog.show()
    }

    private fun openSession(sessionId: String) {
        Log.i(TAG, "openSession id=$sessionId serverUrl=$serverUrl")
        MetricsReporter.recordSessionView()
        currentSessionId = sessionId
        sessionViewModel.stopPolling()
        penOverlay?.destroy()
        penOverlay = null
        annotationModeActive = false
        btnAnnotations.alpha = 0.35f
        btnAnnotations.textSize = 26f
        findViewById<View>(R.id.dotAnnotations).visibility = View.GONE
        currentContextGroupId = null
        contextBar.visibility = View.GONE
        // Force the UI back to draw mode. setToolMode is a no-op if already DRAW.
        toolMode = ToolMode.DRAW
        setModeButtonActive(btnLink, R.id.dotLink, false)
        setModeButtonActive(btnSelect, R.id.dotSelect, false)
        drawControls.visibility = View.VISIBLE
        styleButtons.forEach { it.isEnabled = true }
        selectStyle(selectedStyleIndex)
        wsClient?.disconnect()
        loadedVersion = 0
        MetricsReporter.recordWsConnection()
        wsClient = WebSocketClient(
            serverUrl = serverUrl,
            sessionId = sessionId,
            onMessage = { type, json -> handleWsMessage(type, json) },
            onClosed = { Log.w(TAG, "ws onClosed session=$sessionId") },
        ).also { it.connect() }
        sessionListContainer.visibility = View.GONE
        webView.visibility = View.VISIBLE
        strokeView.visibility = View.VISIBLE
        penToolbar.visibility = View.VISIBLE
        val cachedHtml = docCache.loadHtml(sessionId)
        if (cachedHtml != null) {
            webView.loadDataWithBaseURL("$serverUrl/session/$sessionId", cachedHtml, "text/html", "utf-8", null)
        } else {
            webView.loadUrl("$serverUrl/session/$sessionId")
        }
        val buf = StrokeBuffer()
        sessionRepo.loadStrokes(sessionId, buf)
        val ocrClient = OcrClient { serverUrl }
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
        resetInactivityTimer()
    }

    private fun updateOcrProgress(remaining: Int, total: Int) {
        if (remaining == 0) {
            ocrOverlay.visibility = View.GONE
        } else {
            ocrStatus.text = "OCR $remaining / $total"
            ocrOverlay.visibility = View.VISIBLE
        }
    }

    private fun showSessionList() {
        Log.i(TAG, "showSessionList: leaving session=$currentSessionId")
        inactivityJob?.cancel()
        inactivityJob = null
        wsInSleep = false
        currentContextGroupId = null
        // Reset UI mode so a stale TAG/MOVE state doesn't leak into the next session.
        toolMode = ToolMode.DRAW
        setModeButtonActive(btnLink, R.id.dotLink, false)
        setModeButtonActive(btnSelect, R.id.dotSelect, false)
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
            scope.launch {
                try {
                    val pngData = overlay.exportToPng()
                    val strokeJson = overlay.exportStrokeJson()
                    val annotationsJson = submissionManager.buildAnnotationsJson(
                        overlay.buf.strokes, overlay.bindGroups, elements
                    )
                    val result = withContext(Dispatchers.IO) {
                        submissionManager.submit(serverUrl, sessionId, annotationsJson, pngData, strokeJson)
                    }
                    when {
                        result.isSuccessful -> {
                            MetricsReporter.recordSubmission(true)
                            scope.launch { MetricsReporter.push(pushgatewayUrl) }
                            sessionRepo.clearSession(sessionId)
                            Toast.makeText(this@MainActivity, "Submitted!", Toast.LENGTH_SHORT).show()
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
                } catch (e: Exception) {
                    Log.e(TAG, "submitAndGoBack: exception session=$sessionId", e)
                    Toast.makeText(this@MainActivity, "Error: ${e.message}", Toast.LENGTH_SHORT).show()
                }
            }
        }
    }

    private fun checkVersionAndReload(sessionId: String) {
        scope.launch {
            try {
                val serverVersion = withContext(Dispatchers.IO) {
                    val req = Request.Builder().url("$serverUrl/api/sessions/$sessionId").build()
                    val resp = client.newCall(req).execute()
                    if (!resp.isSuccessful) return@withContext -1
                    val body = resp.body?.string() ?: return@withContext -1
                    org.json.JSONObject(body).optInt("version", -1)
                }
                if (serverVersion > loadedVersion) {
                    Log.i(TAG, "reconnect: server version=$serverVersion > loaded=$loadedVersion — reloading")
                    loadedVersion = serverVersion
                    webView.loadUrl("$serverUrl/session/$sessionId")
                } else {
                    Log.d(TAG, "reconnect: version up to date ($serverVersion)")
                }
            } catch (_: Exception) {}
        }
    }

    private fun requestUpdate() {
        val sessionId = currentSessionId ?: return
        val overlay = penOverlay ?: return
        overlay.queryElementMap { elements ->
            scope.launch {
                try {
                    ocrStatus.text = "Sending annotations..."
                    ocrOverlay.visibility = View.VISIBLE
                    val annotationsJson = submissionManager.buildAnnotationsJson(
                        overlay.buf.strokes, overlay.bindGroups, elements
                    )
                    annotationSentAt = SystemClock.elapsedRealtime()
                    val ok = withContext(Dispatchers.IO) {
                        submissionManager.requestUpdate(serverUrl, sessionId, annotationsJson)
                    }
                    if (!ok) {
                        annotationSentAt = 0L
                        ocrOverlay.visibility = View.GONE
                        Toast.makeText(this@MainActivity, "Request failed", Toast.LENGTH_SHORT).show()
                    }
                } catch (e: Exception) {
                    annotationSentAt = 0L
                    Log.e(TAG, "requestUpdate: exception session=$sessionId", e)
                    ocrOverlay.visibility = View.GONE
                    Toast.makeText(this@MainActivity, "Error: ${e.message}", Toast.LENGTH_SHORT).show()
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
        if (currentSessionId != null) {
            penOverlay?.onResumed()
        }
    }

    override fun onPause() {
        penOverlay?.onPaused()
        super.onPause()
    }

    override fun onDestroy() {
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
            NotificationManager.IMPORTANCE_HIGH,
        ).apply { enableVibration(true) }
        getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
    }

    private fun notifyDocumentUpdated() {
        RingtoneManager.getRingtone(
            this,
            RingtoneManager.getDefaultUri(RingtoneManager.TYPE_NOTIFICATION),
        )?.play()
        @Suppress("DEPRECATION")
        (getSystemService(VIBRATOR_SERVICE) as? Vibrator)?.vibrate(
            VibrationEffect.createOneShot(200, VibrationEffect.DEFAULT_AMPLITUDE),
        )
        val notification = Notification.Builder(this, NOTIFICATION_CHANNEL_ID)
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setContentTitle("Document updated")
            .setContentText("Your e-ink document has been rewritten.")
            .setAutoCancel(true)
            .build()
        getSystemService(NotificationManager::class.java)
            .notify(NOTIFICATION_ID_UPDATE, notification)
    }

    private fun handleWsMessage(type: String, json: JSONObject) {
        Log.d(TAG, "handleWsMessage type=$type session=$currentSessionId json=${json.toString().take(200)}")
        runOnUiThread {
            when (type) {
                "version_updated" -> {
                    val version = json.optInt("version")
                    val sid = currentSessionId ?: return@runOnUiThread
                    val sentAt = annotationSentAt
                    if (sentAt != 0L) {
                        val roundTripMs = SystemClock.elapsedRealtime() - sentAt
                        annotationSentAt = 0L
                        MetricsReporter.recordUpdateRoundTrip(roundTripMs)
                        Log.i(TAG, "version_updated: round-trip=${roundTripMs}ms")
                    }
                    Log.i(TAG, "version_updated: version=$version — reloading webview")
                    loadedVersion = version
                    ocrOverlay.visibility = View.GONE
                    savedScrollY = webView.scrollY
                    webView.loadUrl("$serverUrl/session/$sid")
                    Toast.makeText(this, "Document updated ✓", Toast.LENGTH_SHORT).show()
                    notifyDocumentUpdated()
                }
                "session_submitted" -> {
                    Log.i(TAG, "session_submitted: going back to session list")
                    Toast.makeText(this, "Session submitted", Toast.LENGTH_SHORT).show()
                    showSessionList()
                    startPolling()
                }
                "annotation_result" -> {
                    val version = json.optInt("version")
                    Log.i(TAG, "annotation_result: version=$version — waiting for document update")
                    ocrOverlay.visibility = View.GONE
                    Toast.makeText(this, "Annotations sent ✓", Toast.LENGTH_SHORT).show()
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
