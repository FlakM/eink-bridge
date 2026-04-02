package com.flakm.einkbridge

import android.annotation.SuppressLint
import android.app.AlertDialog
import android.content.SharedPreferences
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.os.Bundle
import android.os.Vibrator
import android.util.Log
import android.view.Gravity
import android.view.View
import android.webkit.*
import android.widget.*
import androidx.activity.OnBackPressedCallback
import androidx.appcompat.app.AppCompatActivity
import androidx.recyclerview.widget.LinearLayoutManager
import androidx.recyclerview.widget.RecyclerView
import kotlinx.coroutines.*
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.MultipartBody
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONArray
import org.json.JSONObject
import org.json.JSONTokener
import java.util.concurrent.TimeUnit

private const val TAG = "EinkMain"

class MainActivity : AppCompatActivity() {
    internal lateinit var webView: WebView
    internal lateinit var sessionListContainer: View
    internal lateinit var penToolbar: View
    internal lateinit var sessionList: RecyclerView
    private lateinit var serverInput: EditText
    private lateinit var serverUrlText: TextView
    internal lateinit var emptyState: TextView
    private lateinit var strokeView: StrokeView
    internal lateinit var adapter: SessionAdapter
    private lateinit var prefs: SharedPreferences
    private var penOverlay: PenOverlay? = null
    private lateinit var docCache: DocumentCache
    private var wsClient: WebSocketClient? = null
    private lateinit var ocrOverlay: View
    private lateinit var ocrStatus: TextView
    private val client = OkHttpClient.Builder()
        .connectTimeout(5, TimeUnit.SECONDS)
        .readTimeout(10, TimeUnit.SECONDS)
        .build()

    private val scope = CoroutineScope(Dispatchers.Main + SupervisorJob())
    private var pollJob: Job? = null
    private var serverUrl = ""
    private var currentSessionId: String? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        prefs = getSharedPreferences("eink_bridge", MODE_PRIVATE)
        docCache = DocumentCache(this)
        serverUrl = prefs.getString("server_url", "http://amd-pc:3333") ?: "http://amd-pc:3333"

        webView = findViewById(R.id.webView)
        sessionListContainer = findViewById(R.id.sessionListContainer)
        penToolbar = findViewById(R.id.penToolbar)
        sessionList = findViewById(R.id.sessionList)
        serverInput = findViewById(R.id.serverInput)
        serverUrlText = findViewById(R.id.serverUrl)
        emptyState = findViewById(R.id.emptyState)
        strokeView = findViewById(R.id.strokeView)
        ocrOverlay = findViewById(R.id.processingOverlay)
        ocrStatus = findViewById(R.id.processingStatus)

        serverInput.setText(serverUrl)
        serverUrlText.text = if (serverUrl.isNotEmpty()) "Connected: $serverUrl" else "Not connected"

        adapter = SessionAdapter { session -> openSession(session.id) }
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

        findViewById<Button>(R.id.connectBtn).setOnClickListener {
            val url = serverInput.text.toString().trimEnd('/')
            if (url.isNotEmpty()) {
                serverUrl = url
                prefs.edit().putString("server_url", url).apply()
                serverUrlText.text = "Connecting..."
                serverUrlText.setTextColor(Color.GRAY)
                scope.launch {
                    val reachable = withContext(Dispatchers.IO) {
                        try {
                            val req = Request.Builder().url("$url/api/health").build()
                            client.newCall(req).execute().isSuccessful
                        } catch (_: Exception) { false }
                    }
                    if (reachable) {
                        serverUrlText.text = "Connected: $serverUrl"
                        serverUrlText.setTextColor(Color.parseColor("#006600"))
                        startPolling()
                    } else {
                        serverUrlText.text = "Unreachable: $serverUrl"
                        serverUrlText.setTextColor(Color.parseColor("#CC0000"))
                    }
                }
            }
        }

        if (serverUrl.isNotEmpty()) {
            startPolling()
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
    private var selectedStyleIndex = 0
    internal var bindModeActive = false
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
        strokeSlider = findViewById(R.id.strokeSlider)
        val btnPencil = findViewById<Button>(R.id.btnPencil)
        val btnBrush = findViewById<Button>(R.id.btnBrush)
        val btnUndo = findViewById<Button>(R.id.btnUndo)
        val btnClear = findViewById<Button>(R.id.btnClear)
        btnLink = findViewById(R.id.btnLink)
        btnLink.text = "Bind"
        btnColor = findViewById(R.id.btnColor)
        btnAnnotations = findViewById(R.id.btnAnnotations)

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
            if (bindModeActive) exitBindMode()
            selectStyle(0)
            penOverlay?.setStylePencil()
        }
        btnBrush.setOnClickListener {
            if (bindModeActive) exitBindMode()
            selectStyle(1)
            penOverlay?.setStyleBrush()
        }
        btnEraser.setOnClickListener {
            if (bindModeActive) exitBindMode()
            selectStyle(2)
            penOverlay?.setStyleEraser()
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
        btnLink.setOnClickListener { enterBindMode() }
        btnColor.setOnClickListener { showColorPicker() }
        btnAnnotations.setOnClickListener {
            annotationModeActive = !annotationModeActive
            btnAnnotations.alpha = if (annotationModeActive) 1.0f else 0.35f
            btnAnnotations.textSize = if (annotationModeActive) 42f else 34f
            findViewById<View>(R.id.dotAnnotations).visibility =
                if (annotationModeActive) View.VISIBLE else View.GONE
            penOverlay?.annotationMode = annotationModeActive
        }
        findViewById<Button>(R.id.btnSubmit).setOnClickListener { submitAndGoBack() }

        selectStyle(0)
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

    internal fun enterBindMode() {
        bindModeActive = true
        btnLink.alpha = 1.0f
        btnLink.textSize = 42f
        findViewById<View>(R.id.dotLink).visibility = View.VISIBLE
        styleDotIds.forEach { findViewById<View>(it).visibility = View.GONE }
        styleButtons.forEach { it.isEnabled = false; it.alpha = 0.2f }
        strokeSlider.isEnabled = false
        val overlay = penOverlay ?: return
        overlay.onBindComplete = { exitBindMode() }
        overlay.enterBindMode()
    }

    internal fun exitBindMode() {
        bindModeActive = false
        btnLink.alpha = 0.35f
        btnLink.textSize = 34f
        findViewById<View>(R.id.dotLink).visibility = View.GONE
        styleButtons.forEach { it.isEnabled = true }
        selectStyle(selectedStyleIndex)
        strokeSlider.isEnabled = true
        penOverlay?.exitBindMode()
    }

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
        currentSessionId = sessionId
        pollJob?.cancel()
        penOverlay?.destroy()
        penOverlay = null
        annotationModeActive = false
        btnAnnotations.alpha = 0.35f
        btnAnnotations.textSize = 34f
        findViewById<View>(R.id.dotAnnotations).visibility = View.GONE
        wsClient?.disconnect()
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
        loadStrokes(sessionId, buf)
        val ocrClient = OcrClient { serverUrl }
        val overlay = PenOverlay(webView, penToolbar, strokeView, buf = buf,
            onStrokesChanged = { saveStrokes(sessionId, buf) },
        )
        overlay.ocrManager = OcrManager(
            recognize = ocrClient::recognize,
            scope = scope,
            onGroupRecognized = { groupId, text -> overlay.onGroupOcrResult(groupId, text) },
            onUnboundResults = { results -> overlay.onUnboundOcrResults(results) },
            onPendingChanged = { remaining, total -> updateOcrProgress(remaining, total) },
        )
        overlay.init()
        overlay.setStrokeColor(currentStrokeColor)
        overlay.onBindGroupsChanged = { saveBindGroups(sessionId, overlay.bindGroups) }
        overlay.onOcrResultsChanged = { results -> saveOcrResults(sessionId, results) }
        overlay.onDeleteBindGroup = { group ->
            AlertDialog.Builder(this)
                .setTitle("Remove link?")
                .setMessage("Unlink ${group.strokeIndices.size} stroke(s) and ${group.elementIndices.size} element(s)?")
                .setPositiveButton("Remove") { _, _ ->
                    overlay.removeBindGroup(group.id)
                    saveBindGroups(sessionId, overlay.bindGroups)
                }
                .setNegativeButton("Cancel", null)
                .show()
        }
        val savedBindGroups = loadBindGroups(sessionId)
        if (savedBindGroups.isNotEmpty()) overlay.loadBindGroups(savedBindGroups)
        val savedOcrResults = loadOcrResults(sessionId)
        if (savedOcrResults.isNotEmpty()) overlay.loadOcrResults(savedOcrResults)
        penOverlay = overlay
    }

    private fun saveStrokes(sessionId: String, buf: StrokeBuffer) {
        val key = "strokes_$sessionId"
        if (buf.isEmpty) prefs.edit().remove(key).apply()
        else prefs.edit().putString(key, buf.toJson()).apply()
    }

    private fun loadStrokes(sessionId: String, buf: StrokeBuffer) {
        val json = prefs.getString("strokes_$sessionId", null) ?: return
        try { buf.loadJson(json) } catch (_: Exception) {}
    }

    private fun saveBindGroups(sessionId: String, groups: List<BindGroup>) {
        val key = "binds_$sessionId"
        if (groups.isEmpty()) prefs.edit().remove(key).apply()
        else prefs.edit().putString(key, bindGroupsToJson(groups)).apply()
    }

    private fun loadBindGroups(sessionId: String): List<BindGroup> {
        val json = prefs.getString("binds_$sessionId", null) ?: return emptyList()
        return try { bindGroupsFromJson(json) } catch (_: Exception) { emptyList() }
    }

    private fun saveOcrResults(sessionId: String, results: List<OcrResult>) {
        val key = "ocr_$sessionId"
        if (results.isEmpty()) prefs.edit().remove(key).apply()
        else prefs.edit().putString(key, ocrResultsToJson(results)).apply()
    }

    private fun loadOcrResults(sessionId: String): List<OcrResult> {
        val json = prefs.getString("ocr_$sessionId", null) ?: return emptyList()
        return try { ocrResultsFromJson(json) } catch (_: Exception) { emptyList() }
    }

    private fun updateOcrProgress(remaining: Int, total: Int) {
        if (remaining == 0) {
            ocrOverlay.visibility = View.GONE
        } else {
            ocrStatus.text = "OCR $remaining / $total"
            ocrOverlay.visibility = View.VISIBLE
        }
    }

    private fun clearSavedStrokes(sessionId: String) {
        prefs.edit().remove("strokes_$sessionId").remove("binds_$sessionId").remove("ocr_$sessionId").apply()
    }

    private fun hasStrokes(sessionId: String): Boolean {
        return prefs.contains("strokes_$sessionId")
    }

    private fun showSessionList() {
        Log.i(TAG, "showSessionList: leaving session=$currentSessionId")
        penOverlay?.disableDrawing()
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
            Log.d(TAG, "submitAndGoBack: queryElementMap callback elements=${elements.size}")
            scope.launch {
                try {
                    val pngData = overlay.exportToPng()
                    Log.d(TAG, "submitAndGoBack: png exported size=${pngData?.size ?: 0}")
                    val strokeJson = overlay.exportStrokeJson()
                    val (explicitGroups, unanchoredStrokes) = bindGroupsToAnnotations(
                        overlay.buf.strokes, overlay.bindGroups
                    )
                    val (proximityGroups, trulyUnanchored) = groupStrokesWithProximity(
                        unanchoredStrokes, elements
                    )
                    val allGroups = explicitGroups + proximityGroups
                    Log.i(TAG, "submitAndGoBack: explicit=${explicitGroups.size} proximity=${proximityGroups.size} unanchored=${trulyUnanchored.size}")
                    val annotationsJson = annotationsToJson(allGroups, trulyUnanchored)

                    withContext(Dispatchers.IO) {
                        val builder = MultipartBody.Builder()
                            .setType(MultipartBody.FORM)
                            .addFormDataPart("typed_notes", "")
                            .addFormDataPart("annotations", annotationsJson)

                        if (pngData != null) {
                            builder.addFormDataPart(
                                "annotation", "strokes.png",
                                pngData.toRequestBody("image/png".toMediaType())
                            )
                        }
                        if (strokeJson != null) {
                            builder.addFormDataPart("stroke_data", strokeJson)
                        }

                        val url = "$serverUrl/api/sessions/$sessionId/submit"
                        Log.i(TAG, "submitAndGoBack: POST $url")
                        val request = Request.Builder().url(url).post(builder.build()).build()

                        val response = client.newCall(request).execute()
                        Log.i(TAG, "submitAndGoBack: response code=${response.code} session=$sessionId")
                        withContext(Dispatchers.Main) {
                            when {
                                response.isSuccessful -> {
                                    Log.i(TAG, "submitAndGoBack: success, clearing strokes and going back")
                                    clearSavedStrokes(sessionId)
                                    Toast.makeText(this@MainActivity, "Submitted!", Toast.LENGTH_SHORT).show()
                                    showSessionList()
                                    startPolling()
                                }
                                response.code == 409 -> {
                                    Log.w(TAG, "submitAndGoBack: 409 conflict session=$sessionId")
                                    Toast.makeText(this@MainActivity, "Session already submitted or expired", Toast.LENGTH_LONG).show()
                                    showSessionList()
                                    startPolling()
                                }
                                else -> {
                                    val body = response.body?.string() ?: ""
                                    Log.e(TAG, "submitAndGoBack: failed code=${response.code} body=$body")
                                    Toast.makeText(this@MainActivity, "Submit failed: ${response.code}", Toast.LENGTH_SHORT).show()
                                }
                            }
                        }
                    }
                } catch (e: Exception) {
                    Log.e(TAG, "submitAndGoBack: exception session=$sessionId", e)
                    withContext(Dispatchers.Main) {
                        Toast.makeText(this@MainActivity, "Error: ${e.message}", Toast.LENGTH_SHORT).show()
                    }
                }
            }
        }
    }

    private fun clearFinishedSessions() {
        scope.launch {
            val ok = withContext(Dispatchers.IO) {
                try {
                    val request = Request.Builder()
                        .url("$serverUrl/api/sessions")
                        .delete()
                        .build()
                    client.newCall(request).execute().isSuccessful
                } catch (_: Exception) { false }
            }
            if (ok) {
                fetchSessions()
            } else {
                Toast.makeText(this@MainActivity, "Failed to clear sessions", Toast.LENGTH_SHORT).show()
            }
        }
    }

    private fun startPolling() {
        pollJob?.cancel()
        pollJob = scope.launch {
            while (isActive) {
                fetchSessions()
                delay(5000)
            }
        }
    }

    private suspend fun fetchSessions() {
        withContext(Dispatchers.IO) {
            try {
                val request = Request.Builder()
                    .url("$serverUrl/api/sessions")
                    .build()
                val response = client.newCall(request).execute()
                if (response.isSuccessful) {
                    val body = response.body?.string() ?: "[]"
                    val arr = JSONArray(body)
                    val sessions = mutableListOf<SessionInfo>()
                    for (i in 0 until arr.length()) {
                        val obj = arr.getJSONObject(i)
                        sessions.add(SessionInfo(
                            id = obj.getString("id"),
                            title = obj.optString("title", "(untitled)"),
                            status = obj.getString("status"),
                            createdAt = obj.getString("created_at"),
                            updatedAt = obj.optString("updated_at", obj.getString("created_at")),
                        ))
                    }
                    docCache.saveSessions(sessions)
                    val hadSessions = adapter.itemCount
                    val pending = sessions.filter { hasStrokes(it.id) }.map { it.id }.toSet()
                    val cached = sessions.filter { docCache.hasCached(it.id) }.map { it.id }.toSet()
                    withContext(Dispatchers.Main) {
                        adapter.setPendingStrokes(pending)
                        adapter.setCachedSessions(cached)
                        adapter.submitList(sessions)
                        if (sessions.isEmpty()) {
                            sessionList.visibility = View.GONE
                            emptyState.visibility = View.VISIBLE
                        } else {
                            sessionList.visibility = View.VISIBLE
                            emptyState.visibility = View.GONE
                        }
                        if (sessions.size > hadSessions && hadSessions > 0) {
                            @Suppress("DEPRECATION")
                            (getSystemService(VIBRATOR_SERVICE) as? Vibrator)?.vibrate(200)
                        }
                        serverUrlText.text = "Connected: $serverUrl"
                        serverUrlText.setTextColor(Color.parseColor("#006600"))
                    }
                }
            } catch (_: Exception) {
                withContext(Dispatchers.Main) {
                    val cached = docCache.loadSessions()
                    if (cached.isNotEmpty()) {
                        serverUrlText.text = "Offline \u2014 showing cached: $serverUrl"
                        serverUrlText.setTextColor(Color.parseColor("#FF9800"))
                        val cachedIds = cached.filter { docCache.hasCached(it.id) }.map { it.id }.toSet()
                        val pending = cached.filter { hasStrokes(it.id) }.map { it.id }.toSet()
                        adapter.setPendingStrokes(pending)
                        adapter.setCachedSessions(cachedIds)
                        adapter.submitList(cached)
                        sessionList.visibility = View.VISIBLE
                        emptyState.visibility = View.GONE
                    } else {
                        serverUrlText.text = "Unreachable: $serverUrl"
                        serverUrlText.setTextColor(Color.parseColor("#CC0000"))
                    }
                }
            }
        }
    }

    override fun onResume() {
        super.onResume()
        if (currentSessionId != null) {
            penOverlay?.enableDrawing()
        }
    }

    override fun onPause() {
        penOverlay?.disableDrawing()
        super.onPause()
    }

    override fun onDestroy() {
        penOverlay?.destroy()
        wsClient?.disconnect()
        scope.cancel()
        super.onDestroy()
    }

    private fun handleWsMessage(type: String, json: JSONObject) {
        Log.d(TAG, "handleWsMessage type=$type session=$currentSessionId json=${json.toString().take(200)}")
        runOnUiThread {
            when (type) {
                "version_updated" -> {
                    val version = json.optInt("version")
                    Log.i(TAG, "version_updated: version=$version — reloading webview")
                    webView.reload()
                    Toast.makeText(this, "Document updated ✓", Toast.LENGTH_SHORT).show()
                }
                "session_submitted" -> {
                    Log.i(TAG, "session_submitted: going back to session list")
                    Toast.makeText(this, "Session submitted", Toast.LENGTH_SHORT).show()
                    showSessionList()
                    startPolling()
                }
                "error" -> {
                    val errMsg = json.optString("message", "Unknown error")
                    Log.w(TAG, "server error: $errMsg")
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
)
