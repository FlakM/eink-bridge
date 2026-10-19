package com.flakm.einkbridge

import okhttp3.Interceptor
import okhttp3.OkHttpClient
import okhttp3.Protocol
import okhttp3.Request
import okhttp3.Response
import okhttp3.ResponseBody.Companion.toResponseBody
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.concurrent.TimeUnit

/**
 * The token is attached in one factory so a new client cannot forget it.
 *
 * The tablet reaches the server over the LAN; a request without the header is a 401 that
 * looks exactly like the server being down.
 */
class HttpClientsTest {

    private class FakeChain(private val request: Request) : Interceptor.Chain {
        var proceeded: Request? = null

        override fun request(): Request = request

        override fun proceed(request: Request): Response {
            proceeded = request
            return Response.Builder()
                .request(request)
                .protocol(Protocol.HTTP_1_1)
                .code(200)
                .message("OK")
                .body("".toResponseBody(null))
                .build()
        }

        override fun connection() = null
        override fun call() = throw UnsupportedOperationException("not called")
        override fun connectTimeoutMillis() = 0
        override fun withConnectTimeout(timeout: Int, unit: TimeUnit) = this
        override fun readTimeoutMillis() = 0
        override fun withReadTimeout(timeout: Int, unit: TimeUnit) = this
        override fun writeTimeoutMillis() = 0
        override fun withWriteTimeout(timeout: Int, unit: TimeUnit) = this
    }

    private fun headerFor(token: String?, url: String = "http://amd-pc:3333/api/sessions"): String? {
        val client = HttpClients.withToken(OkHttpClient(), { token })
        val chain = FakeChain(Request.Builder().url(url).build())
        client.interceptors.single().intercept(chain)
        return chain.proceeded!!.header("Authorization")
    }

    @Test fun aStoredTokenBecomesABearerHeader() {
        assertEquals("Bearer s3cret", headerFor("s3cret"))
    }

    @Test fun noTokenLeavesTheRequestUntouched() {
        assertNull(headerFor(null))
        assertNull(headerFor("   "))
    }

    @Test fun theWebSocketCarriesTheTokenInTheQuery() {
        assertEquals(
            "ws://amd-pc:3333/ws/abc?token=s3cret",
            HttpClients.websocketUrl("http://amd-pc:3333", "abc", "s3cret"),
        )
        assertEquals(
            "ws://amd-pc:3333/ws/abc",
            HttpClients.websocketUrl("http://amd-pc:3333/", "abc", null),
        )
        assertTrue(
            "a token with URL characters must be encoded",
            HttpClients.websocketUrl("http://amd-pc:3333", "abc", "a b&c")
                .endsWith("?token=a+b%26c"),
        )
    }
}
