package com.flakm.einkbridge

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The app's error counts reach the world through this text. One malformed line makes the
 * pushgateway reject the whole batch, and the push is fire-and-forget -- so a broken body
 * is silent, and the errors it was meant to report disappear with it.
 */
class MetricsReporterTest {
    private fun sample(text: String, name: String): Double? =
        text.lines()
            .firstOrNull { it.startsWith("$name{") || it.startsWith("$name ") }
            ?.substringAfterLast(' ')
            ?.toDoubleOrNull()

    @Test
    fun everyLineIsAWellFormedSample() {
        MetricsReporter.recordOcr(true)
        MetricsReporter.recordOcrDuration(120)
        val text = MetricsReporter.metricsText()
        assertTrue(text.isNotEmpty())

        for (line in text.lines()) {
            if (line.isBlank() || line.startsWith("#")) continue
            val splitAt = line.lastIndexOf(' ')
            val sample = line.substring(0, splitAt)
            val value = line.substring(splitAt + 1)
            assertTrue("a sample needs a name: '$line'", sample.isNotBlank())
            assertTrue("the value must be a number: '$line'", value.toDoubleOrNull() != null)
            if (sample.contains('{')) {
                assertTrue("labels must be closed: '$line'", sample.endsWith("}"))
            }
        }
        // Every family that appears has a HELP and a TYPE line.
        val families = text.lines().filter { it.isNotBlank() && !it.startsWith("#") }
            .map { it.substringBefore('{').substringBefore(' ') }
            .toSet()
        for (family in families) {
            assertTrue("no HELP for $family", text.contains("# HELP $family "))
            assertTrue("no TYPE for $family", text.contains("# TYPE $family "))
        }
    }

    @Test
    fun anErrorIsCountedRatherThanOnlyLogged() {
        val before = sample(MetricsReporter.metricsText(), "eink_android_submissions_total{result=\"error\"}") ?: 0.0
        MetricsReporter.recordSubmission(false)
        val after = sample(MetricsReporter.metricsText(), "eink_android_submissions_total{result=\"error\"}")!!
        assertEquals("a failed submission has to reach the counter", before + 1, after, 0.0)
    }

    @Test
    fun anOcrFailureAndSuccessAreCountedSeparately() {
        val beforeOk = sample(MetricsReporter.metricsText(), "eink_android_ocr_requests_total{result=\"ok\"}") ?: 0.0
        val beforeErr = sample(MetricsReporter.metricsText(), "eink_android_ocr_requests_total{result=\"error\"}") ?: 0.0
        MetricsReporter.recordOcr(true)
        MetricsReporter.recordOcr(false)
        val text = MetricsReporter.metricsText()
        assertEquals(beforeOk + 1, sample(text, "eink_android_ocr_requests_total{result=\"ok\"}")!!, 0.0)
        assertEquals(beforeErr + 1, sample(text, "eink_android_ocr_requests_total{result=\"error\"}")!!, 0.0)
    }

    @Test
    fun durationFamiliesOnlyAppearOnceSomethingWasMeasured() {
        // A zero average would read as "instant", which is not the same as "no samples".
        val text = MetricsReporter.metricsText()
        val count = sample(text, "eink_android_ocr_duration_count")
        if (count == null) {
            assertFalse(
                "an average without samples is a lie",
                text.contains("eink_android_ocr_duration_ms_avg")
            )
        } else {
            assertTrue("the count cannot be negative", count >= 0)
        }
    }

    @Test
    fun recordingADurationMovesTheAverageAndTheMax() {
        MetricsReporter.recordOcrDuration(1000)
        MetricsReporter.recordOcrDuration(3000)
        val text = MetricsReporter.metricsText()
        val max = sample(text, "eink_android_ocr_duration_ms_max")
        assertTrue("the max should be at least the largest sample", max != null && max >= 3000)
        val avg = sample(text, "eink_android_ocr_duration_ms_avg")
        assertTrue("the average should be a real average", avg != null && avg in 1.0..max!!)
    }
}
