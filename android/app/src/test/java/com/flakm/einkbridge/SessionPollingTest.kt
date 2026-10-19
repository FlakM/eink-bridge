package com.flakm.einkbridge

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * The list is polled for as long as it is on screen. While the server is away the
 * interval has to grow, and the moment it is back the interval has to be short again,
 * or the reader waits a minute for a list that is already there.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class SessionPollingTest {
    @Test
    fun theDelayDoublesFromFiveSecondsAndCapsAtAMinute() {
        assertEquals(
            listOf(5_000L, 5_000L, 10_000L, 20_000L, 40_000L, 60_000L, 60_000L, 60_000L),
            (0..7).map(::pollDelay),
        )
    }

    @Test
    fun pollingBacksOffWhileFetchesFailAndResetsOnTheFirstSuccess() = runTest {
        val outcomes = ArrayDeque(listOf(true, false, false, false, false, false, true, false))
        val waits = mutableListOf<Long>()
        val job = launch {
            pollWithBackoff(
                fetch = { outcomes.removeFirstOrNull() ?: true },
                wait = { waits += it; delay(it) },
            )
        }
        advanceTimeBy(10 * 60_000L)
        runCurrent()
        job.cancel()
        assertEquals(
            listOf(5_000L, 5_000L, 10_000L, 20_000L, 40_000L, 60_000L, 5_000L, 5_000L),
            waits.take(8),
        )
    }

    @Test
    fun aReachableServerIsAskedEveryFiveSeconds() = runTest {
        var fetches = 0
        val job = launch { pollWithBackoff(fetch = { fetches++; true }) }
        advanceTimeBy(60_000L)
        runCurrent()
        job.cancel()
        assertEquals(13, fetches)
    }
}
