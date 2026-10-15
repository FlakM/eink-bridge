package com.flakm.einkbridge

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class OcrClientTest {
    @Test
    fun `OCR read timeout outlasts server model timeout`() {
        val client = OcrClient.buildHttpClient()
        assertTrue(client.readTimeoutMillis > 120_000)
        assertEquals(3_000, client.connectTimeoutMillis)
    }

    @Test
    fun `OCR call has a bounded total lifetime`() {
        val client = OcrClient.buildHttpClient()
        assertTrue(client.callTimeoutMillis > client.readTimeoutMillis)
        assertTrue(client.callTimeoutMillis <= 135_000)
        assertTrue(OCR_OPERATION_TIMEOUT_MS > client.callTimeoutMillis)
    }
}
