package com.flakm.einkbridge

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The list is bounded now, and the paging policy is the part that can silently lose a
 * review: a wrong URL or a loop that stops early shows an incomplete list.
 */
class SessionPagingTest {
    @Test
    fun theFirstPageAsksForABoundedNumberOfSessions() {
        val url = sessionsPageUrl("http://amd-pc:3333", null)
        assertEquals(
            "http://amd-pc:3333/api/sessions?limit=${SessionViewModel.SESSION_PAGE_SIZE}",
            url
        )
        assertTrue("a page must be bounded", SessionViewModel.SESSION_PAGE_SIZE in 1..200)
    }

    @Test
    fun aCursorIsCarriedAndEscaped() {
        val url = sessionsPageUrl("http://amd-pc:3333", "1-1700000000000000-abc123")
        assertTrue(url.endsWith("&cursor=1-1700000000000000-abc123"))

        // The cursor is opaque to the client, so it has to survive characters that are
        // meaningful in a query string.
        val escaped = sessionsPageUrl("http://amd-pc:3333", "a&b=c d")
        assertTrue("ampersand must not split the query: $escaped", "a%26b%3Dc+d" in escaped || "a%26b%3Dc%20d" in escaped)
    }

    @Test
    fun aRefreshFollowsABoundedNumberOfPages() {
        // A server that keeps handing out cursors must not keep the tablet spinning.
        assertTrue(SessionViewModel.SESSION_PAGE_LIMIT in 1..100)
    }

    @Test
    fun aFailedRefreshKeepsTheListRatherThanBlankingIt() {
        // A transient failure on the first page used to show an empty list and, worse,
        // let the empty result stand in as the offline list.
        assertEquals(ListOutcome.UseCached, listOutcome(fetchedCount = 0, complete = false))
        assertEquals(ListOutcome.UseCached, listOutcome(fetchedCount = 0, complete = true))
    }

    @Test
    fun onlyACompleteWalkIsWorthRemembering() {
        assertEquals(ListOutcome.UseFetchedAndCache, listOutcome(50, complete = true))
        assertEquals(
            "a partial page must not become the offline list",
            ListOutcome.UseFetchedOnly,
            listOutcome(50, complete = false)
        )
    }
}
