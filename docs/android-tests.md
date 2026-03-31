# Android Test Suite

Total: **127 tests**, 0 failures, 3 skipped (E2E tests requiring live server + Claude CLI).

## StrokeBuffer (26 tests)

Pure-Kotlin stroke accumulator. No Android dependencies.

| Test | Rule covered |
|------|-------------|
| `emptyByDefault` | Buffer starts with zero strokes and isEmpty=true |
| `recordsStrokeAfterBeginAddEnd` | begin/addPoint/end sequence produces a committed stroke |
| `recordsStrokeFromBeginEndOnly` | begin+end alone produces a 2-point stroke |
| `accumulatesMultiplePointsInOneStroke` | addPoint calls all land in same stroke |
| `accumulatesMultipleStrokes` | sequential begin/end cycles each produce a separate stroke |
| `undoRemovesLastStroke` | undo() removes the most recently committed stroke |
| `undoOnEmptyIsNoop` | undo() on empty buffer does not throw |
| `undoAllLeavesEmpty` | undoing the only stroke leaves the buffer empty |
| `clearRemovesAllStrokes` | clear() wipes all committed strokes |
| `clearAfterEmptyIsNoop` | clear() on empty buffer does not throw |
| `newStrokeAfterClearWorks` | a new stroke can be committed after clear() |
| `strokesListIsImmutableSnapshot` | strokes property returns a snapshot; subsequent changes do not affect it |
| `strokeWidthIsRecorded` | begin(x, y, width) stores the width in the resulting Stroke |
| `defaultWidthIsThree` | begin(x, y) without explicit width defaults to 3f |
| `twoStrokesHaveDifferentWidths` | two begin calls with different widths store independent widths |
| `previousStrokeWidthUnchangedAfterNewBegin` | starting a new stroke does not mutate a committed stroke's width |
| `eraseRemovesStrokeWithinRadius` | erase() removes strokes whose points fall within radius |
| `eraseDoesNotRemoveStrokeOutsideRadius` | erase() leaves strokes that are beyond radius untouched |
| `eraseEmptyPathReturnsFalse` | erase() with empty path is a no-op returning false |
| `eraseReturnsFalseWhenNothingMatched` | erase() returns false when no stroke was hit |
| `eraseRemovesMultipleMatchingStrokes` | erase() can remove more than one stroke in one call |
| `eraseKeepsStrokesNotInRadius` | erase() only removes matched strokes, leaves others intact |
| `pointsInStrokeMatchInsertionOrder` | points are stored in begin→addPoint→end order |
| `strokeIsImmutableAfterCommit` | snapshot of committed stroke points does not grow with new begin calls |
| `endWithoutBeginNotCommitted` | calling end() before begin() does not commit a stroke |
| `clearDuringInProgressAlsoClearsCurrentPoints` | clear() during an in-progress stroke allows clean restart |

## PenOverlay (45 tests)

Orchestrates the Onyx SDK controller, StrokeBuffer, and StrokeView.

### MockPenController behaviour

| Test | Rule covered |
|------|-------------|
| `simulateStrokeAddsPointsToBuffer` | simulateStroke() injects points into the buffer |
| `mockControllerTracksEnabledState` | setEnabled() toggles drawingEnabled |
| `mockControllerTracksStyle` | setStyleBrush/Pencil flips style field |
| `mockControllerTracksStrokeWidth` | setStrokeWidth() updates lastStrokeWidth |
| `multipleSimulatedStrokesAccumulate` | multiple simulateStroke calls all land in the buffer |

### Rendering

| Test | Rule covered |
|------|-------------|
| `renderStrokesToPngReturnsBytes` | renderStrokesToPng() produces valid PNG bytes (magic header) |
| `renderStrokesToPngSkipsSinglePointStrokes` | single-point strokes do not crash rendering |
| `renderStrokesToPngWithEmptyStrokesList` | empty stroke list produces a blank PNG, not null |
| `exportToPngReturnsNullWhenBufferIsEmpty` | exportToPng() returns null when nothing drawn |
| `exportToPngReturnsBytesAfterSimulatedStroke` | exportToPng() returns bytes after at least one stroke |

### Undo / clear

| Test | Rule covered |
|------|-------------|
| `undoRemovesLastStrokeFromBuffer` | undoLastStroke() removes one stroke from the buffer |
| `clearRemovesAllStrokes` | clearStrokes() empties the buffer |
| `undo_updates_stroke_view_and_resets_render_buffer` | undo syncs StrokeView AND resets the Onyx hardware layer |
| `clear_updates_stroke_view_and_resets_render_buffer` | clear syncs StrokeView AND resets the Onyx hardware layer |
| `multiple_undo_reduces_stroke_view_correctly` | multiple undos reduce StrokeView stroke count one by one |

### Toolbar exclusion

| Test | Rule covered |
|------|-------------|
| `init_toolbarExcludedWhenBothViewsLaidOut` | exclude rect covers the toolbar area when both views are laid out |
| `init_noExcludeRect_whenToolbarHasZeroHeight` | controller does not open until toolbar has a non-zero height |
| `toolbarLayoutChange_updatesExcludeRects` | toolbar resize pushes updated exclude rect to the controller |

### Stroke width does not clear buffer

| Test | Rule covered |
|------|-------------|
| `setStrokeWidth_doesNotClearBuffer` | setStrokeWidth() must not wipe committed strokes from the buffer |
| `setStrokeWidth_updatesCurrentWidthUsedForExport` | after setStrokeWidth, controller receives the new value |

### Per-stroke style preservation

| Test | Rule covered |
|------|-------------|
| `per_stroke_width_is_stored_at_draw_time` | width at time of draw is stored in Stroke, not overwritten later |
| `changing_width_after_stroke_does_not_retroactively_change_previous_stroke` | committed stroke width is immutable |
| `three_strokes_each_retain_their_own_width` | three strokes at three different widths each keep their own width |
| `undo_then_redraw_uses_new_width` | after undo, new stroke picks up the currently active width |

### Scroll / ghost-line bugs

| Test | Rule covered |
|------|-------------|
| `finger_scroll_syncs_stroke_view_before_disabling_sdk` | StrokeView is populated before the SDK is disabled so strokes stay visible during scroll |
| `scroll_calls_reset_render_buffer_on_re_enable_to_prevent_ghost_line` | finger-up calls resetRenderBuffer() to flush Onyx's stored stylus position (prevents ghost line) |
| `finger_scroll_re_syncs_stroke_view_on_finger_up` | after scroll completes, StrokeView shows all strokes |
| `two_strokes_remain_structurally_separate_in_stroke_view_after_scroll` | strokes are not merged or cross-contaminated after a scroll cycle |
| `multiple_scroll_cycles_do_not_lose_strokes` | repeated scroll cycles do not drop strokes from the view |
| `finger_cancel_also_re_enables_drawing` | ACTION_CANCEL (not just ACTION_UP) re-enables drawing |
| `finger_cancel_resets_render_buffer` | ACTION_CANCEL also calls resetRenderBuffer to prevent ghost lines |

### Eraser style

| Test | Rule covered |
|------|-------------|
| `eraser_style_is_tracked_by_mock` | setStyleEraser() sets mock style to "eraser" |
| `style_pencil_after_eraser_reverts_to_pencil` | setStylePencil() after eraser mode restores "pencil" |
| `style_brush_after_eraser_reverts_to_brush` | setStyleBrush() after eraser mode restores "brush" |

### exportStrokeJson

| Test | Rule covered |
|------|-------------|
| `exportStrokeJson_returns_null_when_buffer_empty` | empty buffer → null JSON |
| `exportStrokeJson_contains_canvas_dimensions` | JSON contains canvas_width and canvas_height matching WebView size |
| `exportStrokeJson_contains_all_stroke_points` | JSON stroke array length and per-stroke point counts match the buffer |

### Enable / disable / destroy

| Test | Rule covered |
|------|-------------|
| `disableDrawing_delegates_to_controller` | disableDrawing() calls setEnabled(false) on the controller |
| `enableDrawing_delegates_to_controller` | enableDrawing() calls setEnabled(true) on the controller |
| `destroy_closes_controller` | destroy() calls close() on the controller |
| `stroke_view_not_required_overlay_works_without_it` | overlay is backward-compatible when no StrokeView is supplied |
| `overlay_not_initialized_when_webview_has_no_dimensions` | controller does not open when WebView has zero size |

### StrokeView

| Test | Rule covered |
|------|-------------|
| `strokeView_starts_empty` | new StrokeView has no strokes |
| `strokeView_update_replaces_previous_strokes` | update() replaces the full stroke list, not appends |
| `strokeView_update_with_empty_list_clears_strokes` | update([]) clears all strokes from the view |

## TouchRouting (13 tests)

Pure function `rawDrawingAction` — decides whether to enable/disable the Onyx SDK based on touch input.

| Test | Rule covered |
|------|-------------|
| `fingerDownDisablesRawDrawing` | ACTION_DOWN with finger → disable (return false) |
| `fingerUpEnablesRawDrawing` | ACTION_UP with finger → enable (return true) |
| `fingerCancelEnablesRawDrawing` | ACTION_CANCEL with finger → enable (return true) |
| `fingerMoveIsNoop` | ACTION_MOVE with finger → no action (null) |
| `stylusDownIsNoop` | ACTION_DOWN with stylus → no action (null) |
| `stylusUpIsNoop` | ACTION_UP with stylus → no action (null) |
| `stylusCancelIsNoop` | ACTION_CANCEL with stylus → no action (null) |
| `stylusMoveIsNoop` | ACTION_MOVE with stylus → no action (null) |
| `mixedFingersAndStylusIsNoop` | any pointer being a stylus suppresses the finger logic |
| `multipleFingerDownDisablesRawDrawing` | two-finger ACTION_DOWN also disables |
| `twoFingerUpEnablesRawDrawing` | two-finger ACTION_UP enables |
| `unknownActionWithFingerIsNoop` | unrecognised action code with finger → null |
| `zeroPointerCountFingerDownIsNoop` | zero pointers, no stylus detected → ACTION_DOWN still disables |

## RenderGoldenTest (19 tests)

Golden PNG regression tests for `renderStrokesToPng`. Each test draws a deterministic shape and compares byte-for-byte against a stored PNG in `src/test/snapshots/png/`.

| Test | Shape / scenario |
|------|-----------------|
| `l_shape_3px` | L-shape: vertical + horizontal stroke at 3 px |
| `cross_5px` | + cross: vertical and horizontal bar at 5 px |
| `square_outline_4px` | Square outline: 4 separate edge strokes at 4 px |
| `triangle_3px` | Triangle: 3 edge strokes at 3 px |
| `diagonal_thin_1px` | Single diagonal at 1 px |
| `diagonal_thick_12px` | Single diagonal at 12 px |
| `three_horizontal_lines_rendered_at_2px` | Three parallel horizontals at 2 px |
| `three_horizontal_lines_rendered_at_8px` | Same three horizontals at 8 px (must differ from 2 px golden) |
| `width_change_does_not_alter_render_of_previous_strokes` | Stroke at 3 px + stroke at 10 px — first stroke retains its own width |
| `undo_removes_last_stroke` | 3 horizontals then undo → 2 visible |
| `clear_leaves_blank_canvas` | 3 strokes then clear → blank canvas |
| `undo_all_strokes_one_by_one` | 4 strokes undone one-by-one → blank canvas |
| `lgtm_checkmark_shape` | ✓ checkmark: two connected segments at 5 px |
| `annotated_underline_with_margin_note` | Underline + vertical tick in margin at 3 px |
| `question_mark_shape` | ? shape: arc + stem + dot at 4 px |
| `per_stroke_different_widths_render_correctly` | Thin stroke (2 px) + thick stroke (10 px) in same PNG |
| `single_stroke_thick_15px` | Single horizontal at 15 px |
| `overlapping_strokes_cross` | Horizontal + vertical cross at 3 px |

## SessionAdapter (13 tests across StatusIconTest + FormatSessionTimeTest)

Pure utility functions for list item display.

| Test | Rule covered |
|------|-------------|
| `activeGetsFilledCircle` | Active session → "●" icon |
| `submittedGetsCheckmark` | Submitted session → "✓" icon |
| `cancelledGetsEmptyCircle` | Cancelled session → "○" icon |
| `expiredGetsEmptyCircle` | Expired session → "○" icon |
| `unknownStatusGetsEmptyCircle` | Unknown status → "○" fallback |
| `justNowForLessThanOneMinute` | < 1 min ago → "just now" |
| `minutesAgoForLessThanOneHour` | < 1 hr ago → "N min ago" |
| `hoursAgoForLessThanOneDay` | < 24 hrs ago → "N hr ago" |
| `boundaryAtExactlyOneMinute` | exactly 60 s → "1 min ago" |
| `boundaryAtExactlyOneHour` | exactly 3600 s → "1 hr ago" |
| `formattedDateForOlderThanOneDay` | > 24 hrs ago → date string |
| `malformedIsoFallsBackToFirst16Chars` | malformed ISO string → first 16 chars as fallback |
| `emptyStringFallsBackGracefully` | empty timestamp string → empty fallback, no crash |

## MainActivityTest (9 tests)

Robolectric tests for MainActivity UI state transitions.

| Test | Rule covered |
|------|-------------|
| `sessionListContainerVisibleOnStart` | Session list screen is shown on launch |
| `webViewGoneOnStart` | WebView is hidden on launch |
| `penToolbarGoneOnStart` | Pen toolbar is hidden on launch |
| `emptyStateLabelVisible_whenNoSessions` | "No sessions yet" label shown when adapter is empty |
| `sessionListVisible_whenSessionsExist` | RecyclerView shown (emptyState hidden) when sessions exist |
| `backPressedWhileOnListDoesNotShowWebView` | Back press on session list keeps list visible |
| `connectButtonIsClickable` | Connect button is present and clickable |
| `serverInputShownOnSessionListScreen` | Server URL input field is visible on list screen |
| `penToolbarButtonsExist` | Pencil, brush, eraser, undo, clear, and submit buttons all exist |

## AnnotationE2ETest (3 tests, skipped without env)

Full end-to-end round-trip tests. Skipped unless `ANTHROPIC_API_KEY` is set and a server is running.

| Test | Rule covered |
|------|-------------|
| `submissionWithoutAnnotationIsAccepted` | Session can be submitted with no annotation PNG |
| `annotationPngIsReceivedByServer` | PNG exported from StrokeBuffer is accepted by the server submit endpoint |
| `claudeCliCanDescribeAnnotation` | Claude Vision can describe the annotation content from the submitted PNG |

---

## Regenerating golden PNGs

After intentional rendering changes:

```bash
just golden-android
# expands to:
cd android && UPDATE_GOLDENS=1 nix-shell shell.nix --run './gradlew testDebugUnitTest --tests "*.RenderGoldenTest*"'
```

## Running all tests

```bash
just test-android
# expands to:
cd android && nix-shell shell.nix --run './gradlew testDebugUnitTest'
```
