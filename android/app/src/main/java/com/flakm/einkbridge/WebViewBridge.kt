package com.flakm.einkbridge

import android.webkit.WebView
import org.json.JSONArray
import org.json.JSONObject

internal fun parseFoundElements(raw: String?): List<FoundElement> {
    val cleaned = raw?.trim()?.removeSurrounding("\"")
        ?.replace("\\\\", "\\")?.replace("\\\"", "\"") ?: "[]"
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
    } catch (_: Exception) { emptyList() }
}

internal interface ElementLookup {
    fun findElements(left: Float, top: Float, right: Float, bottom: Float, callback: (List<FoundElement>) -> Unit)
    fun findNearestElement(cx: Float, cy: Float, callback: (List<FoundElement>) -> Unit)
}

internal class WebViewBridge(private val webView: WebView) {

    fun highlightAll() {
        webView.evaluateJavascript("window.__einkHighlightAll && window.__einkHighlightAll()", null)
    }

    fun unhighlightAll() {
        webView.evaluateJavascript("window.__einkUnhighlightAll && window.__einkUnhighlightAll()", null)
    }

    fun setLassoTarget(elementIndex: Int) {
        webView.evaluateJavascript("window.__einkSetLassoTarget && window.__einkSetLassoTarget($elementIndex)", null)
    }

    fun clearLassoTarget() = setLassoTarget(-1)

    fun findElements(left: Float, top: Float, right: Float, bottom: Float, callback: (String?) -> Unit) {
        webView.evaluateJavascript("window.__einkFindElements($left, $top, $right, $bottom)", callback)
    }

    fun findNearestElement(cx: Float, cy: Float, callback: (String?) -> Unit) {
        webView.evaluateJavascript("window.__einkFindNearestElement($cx, $cy)", callback)
    }

    fun applyBindGroups(groups: List<BindGroup>) {
        val groupsJson = JSONArray().apply {
            for (g in groups) {
                put(JSONObject().apply {
                    put("color", "#%06X".format(g.color and 0xFFFFFF))
                    put("indices", JSONArray(g.elementIndices))
                })
            }
        }
        webView.post {
            webView.evaluateJavascript("window.__einkApplyBindGroups && window.__einkApplyBindGroups($groupsJson)", null)
        }
    }

    fun flashGroup(elementIndices: List<Int>, color: Int) {
        val colorHex = "#%06X".format(color and 0xFFFFFF)
        val indicesJson = JSONArray(elementIndices).toString()
        webView.evaluateJavascript("window.__einkFlashGroup($indicesJson, '$colorHex')", null)
    }

    fun computeStrokeLinks() {
        webView.post {
            webView.evaluateJavascript("window.__einkComputeStrokeLinks && window.__einkComputeStrokeLinks([],[])", null)
        }
    }

    fun queryElementMap(callback: (List<ElementEntry>) -> Unit) {
        webView.evaluateJavascript("JSON.stringify(window.__einkElementMap || [])") { json ->
            val cleaned = json?.trim()?.removeSurrounding("\"")
                ?.replace("\\\\", "\\")
                ?.replace("\\\"", "\"") ?: "[]"
            try {
                callback(parseElementMap(cleaned))
            } catch (_: Exception) {
                callback(emptyList())
            }
        }
    }

    fun asElementLookup(): ElementLookup = object : ElementLookup {
        override fun findElements(left: Float, top: Float, right: Float, bottom: Float, callback: (List<FoundElement>) -> Unit) {
            this@WebViewBridge.findElements(left, top, right, bottom) { raw -> callback(parseFoundElements(raw)) }
        }
        override fun findNearestElement(cx: Float, cy: Float, callback: (List<FoundElement>) -> Unit) {
            this@WebViewBridge.findNearestElement(cx, cy) { raw -> callback(parseFoundElements(raw)) }
        }
    }
}
