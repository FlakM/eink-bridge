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
    private var selectedStyleIndex = 0

    private fun setupPenToolbar() {
        strokeSlider = findViewById(R.id.strokeSlider)
        val btnPencil = findViewById<Button>(R.id.btnPencil)
        val btnBrush = findViewById<Button>(R.id.btnBrush)
        val btnUndo = findViewById<Button>(R.id.btnUndo)
        val btnClear = findViewById<Button>(R.id.btnClear)

        val btnEraser = findViewById<Button>(R.id.btnEraser)
        styleButtons.addAll(listOf(btnPencil, btnBrush, btnEraser))

        strokeSlider.setOnSeekBarChangeListener(object : SeekBar.OnSeekBarChangeListener {
            override fun onProgressChanged(seekBar: SeekBar, progress: Int, fromUser: Boolean) {
                penOverlay?.setStrokeWidth(progress.toFloat())
            }
            override fun onStartTrackingTouch(seekBar: SeekBar) {}
            override fun onStopTrackingTouch(seekBar: SeekBar) {}
        })

        btnPencil.setOnClickListener { selectStyle(0); penOverlay?.setStylePencil() }
        btnBrush.setOnClickListener { selectStyle(1); penOverlay?.setStyleBrush() }
        btnEraser.setOnClickListener { selectStyle(2); penOverlay?.setStyleEraser() }
        btnUndo.setOnClickListener { penOverlay?.undoLastStroke() }
        btnClear.setOnClickListener { penOverlay?.clearStrokes() }
        findViewById<Button>(R.id.btnSubmit).setOnClickListener { submitAndGoBack() }

        selectStyle(0)
    }

    private fun selectStyle(idx: Int) {
        selectedStyleIndex = idx
        styleButtons.forEachIndexed { i, btn ->
            btn.alpha = if (i == idx) 1.0f else 0.35f
        }
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
        val overlay = PenOverlay(webView, penToolbar, strokeView)
        overlay.init()
        penOverlay = overlay
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
        scope.launch {
            try {
                val pngData = penOverlay?.exportToPng()
                val strokeJson = penOverlay?.exportStrokeJson()
                withContext(Dispatchers.IO) {
                    val builder = MultipartBody.Builder()
                        .setType(MultipartBody.FORM)
                        .addFormDataPart("typed_notes", "")

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
                    withContext(Dispatchers.Main) {
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
