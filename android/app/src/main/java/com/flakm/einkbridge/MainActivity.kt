package com.flakm.einkbridge

import android.annotation.SuppressLint
import android.content.SharedPreferences
import android.graphics.Color
import android.os.Bundle
import android.os.Vibrator
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
import java.util.concurrent.TimeUnit

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
        serverUrl = prefs.getString("server_url", "http://amd-pc:3333") ?: "http://amd-pc:3333"

        webView = findViewById(R.id.webView)
        sessionListContainer = findViewById(R.id.sessionListContainer)
        penToolbar = findViewById(R.id.penToolbar)
        sessionList = findViewById(R.id.sessionList)
        serverInput = findViewById(R.id.serverInput)
        serverUrlText = findViewById(R.id.serverUrl)
        emptyState = findViewById(R.id.emptyState)
        strokeView = findViewById(R.id.strokeView)

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
    }

    @SuppressLint("SetJavaScriptEnabled")
    private fun setupWebView() {
        webView.settings.javaScriptEnabled = true
        webView.settings.domStorageEnabled = true
        webView.settings.builtInZoomControls = true
        webView.settings.displayZoomControls = false
        webView.settings.useWideViewPort = true
        webView.settings.loadWithOverviewMode = false
    }

    private val styleButtons = mutableListOf<Button>()
    internal lateinit var strokeSlider: SeekBar
    private lateinit var btnLink: Button
    private var selectedStyleIndex = 0
    private var linkMode = false

    @SuppressLint("ClickableViewAccessibility")
    private fun setupPenToolbar() {
        strokeSlider = findViewById(R.id.strokeSlider)
        val btnPencil = findViewById<Button>(R.id.btnPencil)
        val btnBrush = findViewById<Button>(R.id.btnBrush)
        val btnUndo = findViewById<Button>(R.id.btnUndo)
        val btnClear = findViewById<Button>(R.id.btnClear)
        btnLink = findViewById(R.id.btnLink)

        val btnEraser = findViewById<Button>(R.id.btnEraser)
        styleButtons.addAll(listOf(btnPencil, btnBrush, btnEraser))

        strokeSlider.setOnSeekBarChangeListener(object : SeekBar.OnSeekBarChangeListener {
            override fun onProgressChanged(seekBar: SeekBar, progress: Int, fromUser: Boolean) {
                penOverlay?.setStrokeWidth(progress.toFloat())
            }
            override fun onStartTrackingTouch(seekBar: SeekBar) {}
            override fun onStopTrackingTouch(seekBar: SeekBar) {}
        })

        btnPencil.setOnClickListener { exitLinkMode(); selectStyle(0); penOverlay?.setStylePencil() }
        btnBrush.setOnClickListener { exitLinkMode(); selectStyle(1); penOverlay?.setStyleBrush() }
        btnEraser.setOnClickListener { exitLinkMode(); selectStyle(2); penOverlay?.setStyleEraser() }
        btnUndo.setOnClickListener { penOverlay?.undoLastStroke() }
        btnClear.setOnClickListener { penOverlay?.clearStrokes(); penOverlay?.clearExplicitBindings() }
        btnLink.setOnClickListener { toggleLinkMode() }
        findViewById<Button>(R.id.btnSubmit).setOnClickListener { submitAndGoBack() }

        selectStyle(0)
    }

    private fun selectStyle(idx: Int) {
        selectedStyleIndex = idx
        styleButtons.forEachIndexed { i, btn ->
            btn.alpha = if (i == idx) 1.0f else 0.35f
        }
    }

    @SuppressLint("ClickableViewAccessibility")
    private fun toggleLinkMode() {
        linkMode = !linkMode
        btnLink.alpha = if (linkMode) 1.0f else 0.35f
        if (linkMode) {
            penOverlay?.disableDrawing()
            strokeView.setOnTouchListener { _, event ->
                if (event.actionMasked == android.view.MotionEvent.ACTION_UP) {
                    penOverlay?.tapToBindElement(event.x, event.y) { entry, anchored ->
                        val msg = if (entry != null) {
                            if (anchored) "Linked: ${entry.tag} — ${entry.text.take(40)}"
                            else "Unlinked: ${entry.tag}"
                        } else "No element nearby"
                        Toast.makeText(this, msg, Toast.LENGTH_SHORT).show()
                    }
                }
                true
            }
        } else {
            exitLinkMode()
        }
    }

    private fun exitLinkMode() {
        if (!linkMode) return
        linkMode = false
        btnLink.alpha = 0.35f
        strokeView.setOnTouchListener(null)
        penOverlay?.enableDrawing()
    }

    private fun openSession(sessionId: String) {
        currentSessionId = sessionId
        pollJob?.cancel()
        penOverlay?.destroy()
        penOverlay = null
        sessionListContainer.visibility = View.GONE
        webView.visibility = View.VISIBLE
        strokeView.visibility = View.VISIBLE
        penToolbar.visibility = View.VISIBLE
        webView.loadUrl("$serverUrl/session/$sessionId")
        val buf = StrokeBuffer()
        loadStrokes(sessionId, buf)
        val overlay = PenOverlay(webView, penToolbar, strokeView, buf = buf,
            onStrokesChanged = { saveStrokes(sessionId, buf) })
        overlay.init()
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

    private fun clearSavedStrokes(sessionId: String) {
        prefs.edit().remove("strokes_$sessionId").apply()
    }

    private fun hasStrokes(sessionId: String): Boolean {
        return prefs.contains("strokes_$sessionId")
    }

    private fun showSessionList() {
        penOverlay?.disableDrawing()
        penOverlay?.destroy()
        penOverlay = null
        currentSessionId = null
        webView.visibility = View.GONE
        strokeView.visibility = View.GONE
        penToolbar.visibility = View.GONE
        sessionListContainer.visibility = View.VISIBLE
    }

    private fun submitAndGoBack() {
        val sessionId = currentSessionId ?: return
        val overlay = penOverlay ?: return
        overlay.queryElementMap { elements ->
            scope.launch {
                try {
                    val pngData = overlay.exportToPng()
                    val strokeJson = overlay.exportStrokeJson()
                    val (groups, unanchored) = groupStrokesWithProximity(
                        overlay.buf.strokes, elements, overlay.explicitBindings
                    )
                    val annotationsJson = annotationsToJson(groups, unanchored)

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

                        val request = Request.Builder()
                            .url("$serverUrl/api/sessions/$sessionId/submit")
                            .post(builder.build())
                            .build()

                        val response = client.newCall(request).execute()
                        withContext(Dispatchers.Main) {
                            if (response.isSuccessful) {
                                clearSavedStrokes(sessionId)
                                Toast.makeText(this@MainActivity, "Submitted!", Toast.LENGTH_SHORT).show()
                                showSessionList()
                                startPolling()
                            } else {
                                Toast.makeText(this@MainActivity, "Submit failed: ${response.code}", Toast.LENGTH_SHORT).show()
                            }
                        }
                    }
                } catch (e: Exception) {
                    Toast.makeText(this@MainActivity, "Error: ${e.message}", Toast.LENGTH_SHORT).show()
                }
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
                    val hadSessions = adapter.itemCount
                    val pending = sessions.filter { hasStrokes(it.id) }.map { it.id }.toSet()
                    withContext(Dispatchers.Main) {
                        adapter.setPendingStrokes(pending)
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
                    }
                }
            } catch (_: Exception) {
                withContext(Dispatchers.Main) {
                    serverUrlText.text = "Unreachable: $serverUrl"
                    serverUrlText.setTextColor(Color.parseColor("#CC0000"))
                }
            }
        }
    }

    override fun onResume() {
        super.onResume()
        // Re-enable pen drawing if we had an open session before going to background.
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
        scope.cancel()
        super.onDestroy()
    }
}

data class SessionInfo(
    val id: String,
    val title: String,
    val status: String,
    val createdAt: String,
    val updatedAt: String,
)
