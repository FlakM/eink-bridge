package com.flakm.einkbridge

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * The stylus watchdog's decision table. Each row is a way the pen has gone quiet on the
 * tablet: the SDK flag dropped, the native reader wedged, a recovery that did not take.
 */
class PenWatchdogTest {
    private val longAgo = RAW_WATCHDOG_ESCALATE_MS * 10

    @Test fun nothingIsDoneWhileThePenIsLegitimatelyOff() {
        assertEquals(PenRecovery.NONE, penRecovery(rawCallbackSinceDown = false, shouldDraw = false, sdkEnabled = false, msSinceLastRecovery = longAgo))
    }

    @Test fun aReaderThatAnsweredIsLeftAlone() {
        assertEquals(PenRecovery.NONE, penRecovery(rawCallbackSinceDown = true, shouldDraw = true, sdkEnabled = true, msSinceLastRecovery = longAgo))
        assertEquals(PenRecovery.NONE, penRecovery(rawCallbackSinceDown = true, shouldDraw = true, sdkEnabled = false, msSinceLastRecovery = 10))
    }

    @Test fun anSdkThatSaysInputIsOffIsTurnedBackOnWithoutATeardown() {
        assertEquals(PenRecovery.ARM, penRecovery(rawCallbackSinceDown = false, shouldDraw = true, sdkEnabled = false, msSinceLastRecovery = longAgo))
    }

    @Test fun aSilentReaderThatClaimsToBeOnGetsItsRawLayerReopened() {
        assertEquals(PenRecovery.RESET, penRecovery(rawCallbackSinceDown = false, shouldDraw = true, sdkEnabled = true, msSinceLastRecovery = longAgo))
        assertEquals("an SDK that cannot say is treated as on", PenRecovery.RESET, penRecovery(false, true, null, longAgo))
    }

    @Test fun aRecoveryThatDidNotTakeIsEscalatedToARecreate() {
        assertEquals(PenRecovery.RECREATE, penRecovery(rawCallbackSinceDown = false, shouldDraw = true, sdkEnabled = true, msSinceLastRecovery = RAW_WATCHDOG_ESCALATE_MS - 1))
        assertEquals(PenRecovery.RECREATE, penRecovery(rawCallbackSinceDown = false, shouldDraw = true, sdkEnabled = false, msSinceLastRecovery = 0))
        assertEquals("the window has passed: start over gently", PenRecovery.RESET, penRecovery(false, true, true, RAW_WATCHDOG_ESCALATE_MS))
    }
}
