package com.flakm.einkbridge

import org.junit.Assert.*
import org.junit.Test
import java.time.LocalDateTime

class StatusIconTest {
    @Test fun activeGetsFilledCircle() = assertEquals("\u25CF", statusIcon("Active"))
    @Test fun submittedGetsCheckmark() = assertEquals("\u2713", statusIcon("Submitted"))
    @Test fun cancelledGetsEmptyCircle() = assertEquals("\u25CB", statusIcon("Cancelled"))
    @Test fun expiredGetsEmptyCircle() = assertEquals("\u25CB", statusIcon("Expired"))
    @Test fun unknownStatusGetsEmptyCircle() = assertEquals("\u25CB", statusIcon(""))
}

class FormatSessionTimeTest {
    private fun at(minutesAgo: Long): LocalDateTime =
        LocalDateTime.now().minusMinutes(minutesAgo)

    private fun format(iso: String, now: LocalDateTime = LocalDateTime.now()) =
        formatSessionTime(iso, now)

    // Use a fixed "now" and build ISO strings relative to it so tests don't depend on wall clock

    private val now = LocalDateTime.of(2026, 3, 30, 12, 0, 0)
    private fun isoAt(minutesAgo: Long): String =
        now.minusMinutes(minutesAgo)
            .atZone(java.time.ZoneId.systemDefault())
            .toInstant()
            .toString()

    @Test fun justNowForLessThanOneMinute() {
        assertEquals("just now", format(isoAt(0), now))
    }

    @Test fun minutesAgoForLessThanOneHour() {
        val result = format(isoAt(15), now)
        assertEquals("15m ago", result)
    }

    @Test fun hoursAgoForLessThanOneDay() {
        val result = format(isoAt(120), now)
        assertEquals("2h ago", result)
    }

    @Test fun formattedDateForOlderThanOneDay() {
        val result = format(isoAt(60 * 25), now) // 25 hours ago
        // Should be a formatted date, not "Xh ago"
        assertFalse(result.endsWith("h ago"))
        assertTrue(result.isNotEmpty())
    }

    @Test fun malformedIsoFallsBackToFirst16Chars() {
        val bad = "not-an-iso-timestamp"
        assertEquals("not-an-iso-times", format(bad))
    }

    @Test fun emptyStringFallsBackGracefully() {
        val result = format("")
        assertEquals("", result) // empty.take(16) == ""
    }

    @Test fun boundaryAtExactlyOneMinute() {
        // exactly 1 minute ago → "1m ago" (toMinutes() == 1, which is NOT < 1)
        assertEquals("1m ago", format(isoAt(1), now))
    }

    @Test fun boundaryAtExactlyOneHour() {
        // exactly 60 minutes → toMinutes() == 60, NOT < 60 → hours branch
        assertEquals("1h ago", format(isoAt(60), now))
    }
}
