package com.flakm.einkbridge

import okhttp3.OkHttpClient
import java.net.URLEncoder

/**
 * One place where the API token is attached.
 *
 * Every HTTP client in the app is built here so a new client cannot quietly skip the
 * header: the server may be on the LAN, and a request without it is a 401 that looks like
 * an outage from the tablet.
 */
internal object HttpClients {
    /** Every client starts here, so each one finds the server the same way ([ServerDns]). */
    fun builder(): OkHttpClient.Builder = OkHttpClient.Builder().dns(ServerDns.shared)

    fun withToken(base: OkHttpClient, token: () -> String?): OkHttpClient =
        base.newBuilder().addInterceptor { chain ->
            val value = token()?.trim().orEmpty()
            val request = if (value.isEmpty()) {
                chain.request()
            } else {
                chain.request().newBuilder()
                    .header("Authorization", "Bearer $value")
                    .build()
            }
            chain.proceed(request)
        }.build()

    /** The WebSocket upgrade is browser-shaped, so the token travels in the query. */
    /**
     * `client=tablet` tells the server this socket can answer a screenshot request. The
     * agent's own observer shares the socket, so without it "someone is listening" and
     * "the tablet is listening" would be the same thing.
     */
    fun websocketUrl(serverUrl: String, sessionId: String, token: String?): String {
        val base = serverUrl.trimEnd('/')
            .replaceFirst("http://", "ws://")
            .replaceFirst("https://", "wss://")
        val value = token?.trim().orEmpty()
        val tokenParam = if (value.isEmpty()) ""
        else "&token=" + URLEncoder.encode(value, "UTF-8")
        return "$base/ws/$sessionId?client=tablet$tokenParam"
    }
}
