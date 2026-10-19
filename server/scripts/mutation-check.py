#!/usr/bin/env python3
"""Prove the render invariants bite: reintroduce each bug and watch a test go red.

The tests in `src/render/tests.rs` exist because of specific regressions -- a clipped
annotation outline, a canvas that went dark, a steps block that ate the pen's taps. A
green suite only shows the tests pass; it does not show they would notice the bug
coming back. This script applies each defect on its own, asserts the named test fails,
reverts, and finally asserts the suite is green again.

    just test-mutations
    just test-mutations render   # only the cases whose label or test mentions "render"
"""
from __future__ import annotations

import pathlib
import plistlib
import re
import subprocess
import sys

SERVER = pathlib.Path(__file__).resolve().parent.parent
RENDER = SERVER / "src/render.rs"
EINK_CSS = SERVER / "assets/eink.css"
ELEMENT_MAP_JS = SERVER / "assets/element-map.js"
DIAGRAMS = SERVER / "src/render/diagrams.rs"
MARKDOWN = SERVER / "src/render/markdown.rs"
CHUNKS = SERVER / "src/render/chunks.rs"
REANCHOR = SERVER / "src/reanchor.rs"
CLII = SERVER / "src/cli.rs"
APII = SERVER / "src/api.rs"
APPI = SERVER / "src/app.rs"
METRICS = SERVER / "src/metrics.rs"
WSI = SERVER / "src/ws.rs"
OCRI = SERVER / "src/ocr.rs"
PROVIDERSI = SERVER / "src/ocr/providers.rs"
MAINI = SERVER / "src/main.rs"
STROKEBUFFER = SERVER.parent / "android/app/src/main/java/com/flakm/einkbridge/StrokeBuffer.kt"
FLAKE = SERVER.parent / "flake.nix"
SESSIONI = SERVER / "src/session.rs"
SUBMISSION = SERVER.parent / "android/app/src/main/java/com/flakm/einkbridge/SubmissionManager.kt"
SESSIONVM = SERVER.parent / "android/app/src/main/java/com/flakm/einkbridge/SessionViewModel.kt"
PENOVERLAY = SERVER.parent / "android/app/src/main/java/com/flakm/einkbridge/PenOverlay.kt"
THEME = SERVER / "assets/eink-theme.tmTheme"


# Tests in a binary crate are invisible to `--lib`; they run against the bin target.
TARGETS = {
    "a_review_exit_code_is_the_contract_a_pipeline_gates_on": "--bin eink-review",
    "long_element_text_is_elided_without_panicking_on_a_character_boundary": "--bin eink-review",
    "the_observer_url_follows_the_scheme_of_the_server": "--bin eink-review",
    "a_session_row_shows_the_star_the_status_and_where_it_came_from": "--bin eink-review",
    "a_submitted_review_renders_notes_anchors_and_images": "--bin eink-review",
    "the_connection_gauge_counts_up_and_back_down_to_zero": "--test ws_metrics_test",
    "remove_forgets_the_session_and_deletes_its_files": "--test api_edge_test",
    "a_submission_with_a_broken_field_is_told_which_field": "--test api_edge_test",
    "an_acknowledgement_must_name_the_feedback_it_consumes": "--test api_edge_test",
    "metrics_are_served_in_the_format_prometheus_scrapes": "--test api_edge_test",
    "server_side_ocr_is_optional_and_says_so": "--test api_edge_test",
    "cli_remove_takes_the_review_away_and_cancel_keeps_it": "--test cli_integration_test",
    "a_label_value_cannot_break_the_pushed_body": "--bin eink-review",
    "the_webhook_metric_counts_what_actually_happened": "--test api_edge_test",
    "the_cli_pushes_its_own_command_metrics": "--test cli_integration_test",
    "an_annotation_index_cannot_escape_the_session_directory": "--test api_edge_test",
    "a_client_that_stops_answering_is_dropped": "--test ws_heartbeat_test",
    "an_occupied_port_is_reported_in_words": "--test startup_test",
    "the_allowlist_matches_the_filter_in_the_flake": "--test build_source_test",
    "health_says_what_is_actually_wrong": "--test api_edge_test",
    "the_session_list_pages_without_repeating_or_dropping_a_session": "--test api_edge_test",
    "a_page_size_of_zero_or_a_bad_cursor_is_refused": "--test api_edge_test",
    "a_response_means_the_change_is_on_disk": "--test api_edge_test",
    "remove_forgets_the_session_and_deletes_its_files": "--test api_edge_test",
    "the_cloud_timeout_stays_inside_a_usable_range": "--lib",
    "a_local_runtime_http_error_is_reported_with_its_status": "--lib",
    "handwriting_with_nothing_drawn_makes_no_request": "--lib",
    "a_client_that_connects_after_an_update_still_receives_it": "--test ws_test",
    "an_unreadable_message_does_not_break_the_connection": "--test ws_test",
}


def target_for(test: str) -> str:
    # A gradle case names its class in the test slot, since the class *is* the selector.
    if test.startswith("gradle:"):
        return test
    return TARGETS.get(test, "--lib")


class NoTestsRan(RuntimeError):
    """A test name that matches nothing proves nothing: it must not read as a pass."""


ANDROID = SERVER.parent / "android"


def gradle_is_green(class_name: str) -> bool:
    """Run one Kotlin test class; the tablet's logic is tested by gradle, not cargo."""
    proc = subprocess.run(
        ["./gradlew", "testDebugUnitTest", "--tests", f"com.flakm.einkbridge.{class_name}"],
        cwd=ANDROID,
        capture_output=True,
        text=True,
    )
    output = proc.stdout + proc.stderr
    if proc.returncode == 0:
        if "BUILD SUCCESSFUL" not in output:
            raise NoTestsRan(f"{class_name} did not build: {output[-400:]}")
        return True
    if "compilation error" in output.lower() or output.count("e: ") > 0:
        # The mutation did not compile, which is not evidence that a test noticed it.
        raise NoTestsRan(f"{class_name} did not compile with this mutation: {output[-400:]}")
    if "tests completed" not in output:
        raise NoTestsRan(f"{class_name} ran no tests: {output[-400:]}")
    return False


def suite_is_green(test: str = "", target: str = "--lib") -> bool:  # noqa: D401
    if target.startswith("gradle:"):
        return gradle_is_green(target.split(":", 1)[1])
    proc = subprocess.run(
        ["cargo", "test", *target.split(), "--quiet", test],
        cwd=SERVER,
        capture_output=True,
        text=True,
    )
    if test:
        ran = sum(
            int(match.group(1))
            for match in re.finditer(r"test result: \w+\. (\d+) passed", proc.stdout)
        )
        if ran == 0 and proc.returncode == 0:
            raise NoTestsRan(
                f"'{test}' ran no tests in {target}: it lives in another target, so this "
                f"mutation would have looked caught without being run"
            )
    return proc.returncode == 0


def locate(body: str, needle: str) -> tuple[int, int] | None:
    """Find `needle`, ignoring how the formatter wrapped it."""
    if not needle:
        return None
    start = body.find(needle)
    if start >= 0:
        return start, start + len(needle)
    pattern = re.compile(r"\s+".join(re.escape(part) for part in needle.split()))
    match = pattern.search(body)
    return (match.start(), match.end()) if match else None


def text_edit(path: pathlib.Path, old: str, new: str):
    """Swap `old` for `new`, remembering the exact text replaced.

    Reverting writes back what was there, not the anchor as typed, so a run leaves the
    tree byte-identical even when the formatter had rewrapped the code.
    """
    saved: dict[str, object] = {}

    def apply():
        body = path.read_text()
        found = locate(body, old)
        assert found, f"anchor missing in {path}: {old[:70]!r}"
        start, end = found
        saved["original"] = body
        saved["text"] = body[start:end]
        saved["start"] = start
        path.write_text(body[:start] + new + body[end:])

    def revert():
        body = path.read_text()
        # Undo at the offset that was mutated, not at the first match: a replacement that
        # also appears elsewhere (a bare DELETE, say) would otherwise be restored into the
        # wrong function and silently swap two behaviours.
        at = int(saved["start"])
        if new and body[at : at + len(new)] == new:
            path.write_text(body[:at] + str(saved["text"]) + body[at + len(new) :])
        elif not new:
            path.write_text(body[:at] + str(saved["text"]) + body[at:])
        else:
            found = locate(body, new)
            assert found, f"mutation not present in {path}: {new[:70]!r}"
            start, end = found
            path.write_text(body[:start] + str(saved["text"]) + body[end:])
        restored = path.read_text()
        assert restored == saved["original"], (
            f"{path} did not come back byte for byte; the tree is no longer trustworthy"
        )

    return apply, revert


def theme_edit(edit):
    """Apply `edit(doc)`; revert restores exactly what that edit replaced."""
    saved: dict[str, str] = {}

    def apply():
        doc = plistlib.loads(THEME.read_bytes())
        edit(doc, saved)
        THEME.write_bytes(plistlib.dumps(doc))

    def revert():
        doc = plistlib.loads(THEME.read_bytes())
        if "background" in saved:
            doc["settings"][0]["settings"]["background"] = saved["background"]
        if "scope" in saved:
            for entry in doc["settings"]:
                if (entry.get("settings") or {}).get("foreground") == "#ffffff":
                    entry["settings"]["foreground"] = saved["scope"]
                    break
        THEME.write_bytes(plistlib.dumps(doc))

    return apply, revert


def darken_canvas(doc, saved):
    saved["background"] = doc["settings"][0]["settings"]["background"]
    doc["settings"][0]["settings"]["background"] = "#9c2a00"


def whiten_a_scope(doc, saved):
    # `#ffffff` is the trap: on paper it is invisible, and it is the colour a blanket
    # substitution reaches for when fixing an unrelated scope.
    for entry in doc["settings"][1:]:
        foreground = (entry.get("settings") or {}).get("foreground")
        if foreground:
            saved["scope"] = foreground
            entry["settings"]["foreground"] = "#ffffff"
            return
    raise AssertionError("no scope with a foreground to mutate")


CASES = [
    (
        "the snippet wrapper clips its contents again",
        text_edit(
            EINK_CSS,
            """.snippet-block {
  margin: 16px 0;
  border: 1px solid #d0d0d0;
  border-radius: 4px;""",
            """.snippet-block {
  margin: 16px 0;
  border: 1px solid #d0d0d0;
  border-radius: 4px;
  overflow: hidden;""",
        ),
        "only_viewports_clip_their_contents",
    ),
    (
        "the diff block scrolls instead of wrapping",
        text_edit(
            EINK_CSS,
            """.diff-block {
    font-family: 'Roboto Mono', 'Droid Sans Mono', monospace;""",
            """.diff-block {
    overflow-x: auto;
    font-family: 'Roboto Mono', 'Droid Sans Mono', monospace;""",
        ),
        "only_viewports_clip_their_contents",
    ),
    (
        "the diagram block clips the notes <pre> inside it",
        text_edit(
            EINK_CSS,
            """.diagram-block {
    margin: 28px 0;
    border: 2px solid #888;
    border-radius: 10px;
    background: #fff;""",
            """.diagram-block {
    margin: 28px 0;
    border: 2px solid #888;
    border-radius: 10px;
    background: #fff;
    overflow: hidden;""",
        ),
        "content_that_can_be_annotated_is_never_clipped",
    ),
    (
        "the code canvas goes dark",
        theme_edit(darken_canvas),
        "every_syntax_colour_is_readable_on_paper",
    ),
    (
        "a scope is painted paper-white",
        theme_edit(whiten_a_scope),
        "every_syntax_colour_is_readable_on_paper",
    ),
    (
        "a tap anywhere in a steps block advances it",
        text_edit(
            ELEMENT_MAP_JS,
            """      setStep(block, 0);
      var next = block.querySelector('[data-role="step-next"]');""",
            """      setStep(block, 0);
      block.addEventListener('click', function() { advanceStep(block); });
      var next = block.querySelector('[data-role="step-next"]');""",
        ),
        "the_pen_owns_taps_so_a_steps_block_does_not_steal_them",
    ),
    (
        "the steps dim by default rather than by state",
        text_edit(
            EINK_CSS,
            """.steps-list li {
  counter-increment: step;
  padding: 8px 0 8px 34px;
  position: relative;
  opacity: 1;
}""",
            """.steps-list li {
  counter-increment: step;
  padding: 8px 0 8px 34px;
  position: relative;
  opacity: 0.35;
}""",
        ),
        "a_steps_block_still_reads_without_script",
    ),
    (
        "the markup pre-dims the steps",
        text_edit(
            DIAGRAMS,
            '<li><span class=\\"step-title\\">{}</span>',
            '<li data-state=\\"todo\\"><span class=\\"step-title\\">{}</span>',
        ),
        "a_steps_block_still_reads_without_script",
    ),
    (
        "a block kind is left out of the element map",
        text_edit(
            ELEMENT_MAP_JS,
            "var QUERY = 'h1, h2, h3, p, li, pre, blockquote, td, th, table, ul, ol, img, "
            ".diagram-block, .snippet-block, .steps-block, .diff-block';",
            "var QUERY = 'h1, h2, h3, p, li, pre, blockquote, td, th, table, ul, ol, img, "
            ".diagram-block, .snippet-block, .steps-block';",
        ),
        "every_block_kind_can_be_bound_by_a_tap",
    ),
    (
        "the line marks are counted one line high",
        text_edit(
            DIAGRAMS,
            ".strip_prefix('\\n')",
            ".strip_prefix('Z')",
        ),
        "a_line_mark_lands_on_the_line_it_names",
    ),
    (
        "the closing fence must be exactly three backticks",
        text_edit(
            CHUNKS,
            "    let run = backtick_run(line);\n    run >= opening_run && line.trim()[run..].trim().is_empty()",
            "    let _ = opening_run;\n    line.trim() == \"```\"",
        ),
        "parse_chunks_reads_a_shorter_fence_inside_a_longer_one",
    ),
    (
        "a snippet wraps its code in a fence it can contain",
        text_edit(
            DIAGRAMS,
            "let fence = \"`\".repeat(longest_backtick_run(&body).max(3) + usize::from(body.contains('`')));",
            "let fence = \"```\".to_string();",
        ),
        "a_snippet_may_quote_a_fence_of_its_own",
    ),
    (
        "any first token counts as a language",
        text_edit(
            DIAGRAMS,
            r"""    !token.is_empty()
        && !token.contains(['/', '\\', ':', '.', '#'])
        && token
            .chars()
            .all(|c| c.is_alphanumeric() || "+-_".contains(c))""",
            r"""    !token.is_empty()""",
        ),
        "a_header_without_a_language_names_the_file_instead",
    ),
    (
        "the caption is emitted unescaped",
        text_edit(
            DIAGRAMS,
            "            \"<figcaption>{}</figcaption>\",\n            super::html_utils::escape_html(&title)",
            "            \"<figcaption>{}</figcaption>\",\n            title.clone()",
        ),
        "review_text_in_a_snippet_or_step_block_is_escaped",
    ),
    (
        "a single step is described as plural",
        text_edit(
            DIAGRAMS,
            "let noun = if steps.len() == 1 { \"step\" } else { \"steps\" };",
            "let noun = \"steps\";",
        ),
        "a_single_step_is_not_plural",
    ),
    (
        "trailing tag scaffolding is counted as a line",
        text_edit(
            DIAGRAMS,
            "    if pending {\n        lines.push(line);\n    }",
            "    lines.push(line);",
        ),
        "a_marked_block_keeps_one_rendered_line_per_line_of_code",
    ),
    (
        "raw HTML in a brief is written to the page unsanitised",
        text_edit(
            MARKDOWN,
            "    if events.iter().any(is_untrusted) {",
            "    if false {",
        ),
        "a_script_in_a_paragraph_is_dropped",
    ),
    (
        "the payload only escapes a lowercase closing tag",
        text_edit(
            DIAGRAMS,
            """    json.replace('&', "\\\\u0026")
        .replace('<', "\\\\u003c")
        .replace('>', "\\\\u003e")""",
            """    json.replace("</script>", "<\\\\/script>")""",
        ),
        "a_diagram_source_cannot_close_the_payload_script",
    ),
    (
        "a guess may take a paragraph an exact match owns",
        text_edit(
            REANCHOR,
            "            || scores.get(1).is_some_and(|other| score - other.1 < 0.08)\n            || claimed.contains(&index)\n        {",
            "            || scores.get(1).is_some_and(|other| score - other.1 < 0.08)\n        {",
        ),
        "a_relocated_note_never_lands_on_another_paragraph",
    ),
    (
        "the preview is cut at a byte offset",
        text_edit(
            CLII,
            """    if text.chars().count() <= limit {
        return text.to_string();
    }
    let kept: String = text.chars().take(limit).collect();
    format!("{kept}…")""",
            """    if text.len() <= limit {
        return text.to_string();
    }
    text[..limit].to_string()""",
        ),
        "long_element_text_is_elided_without_panicking_on_a_character_boundary",
    ),
    (
        "a review with changes exits zero",
        text_edit(
            CLII,
            '        Some("changes") => 2,',
            '        Some("changes") => 0,',
        ),
        "a_review_exit_code_is_the_contract_a_pipeline_gates_on",
    ),
    (
        "the observer url keeps a trailing slash",
        text_edit(
            CLII,
            "    let base = server\n        .trim_end_matches('/')",
            "    let base = server",
        ),
        "the_observer_url_follows_the_scheme_of_the_server",
    ),
    (
        "ink without rendered images reports itself as empty",
        text_edit(
            APII,
            """            has_annotation: !session.annotation_images.is_empty()
                || !session.annotations.is_empty()
                || session.stroke_data.is_some(),""",
            """            has_annotation: !session.annotation_images.is_empty(),""",
        ),
        "a_review_with_ink_reports_that_it_has_one",
    ),
    (
        "the spec forgets a field the result sends",
        text_edit(
            APII,
            """                        "starred": {"type": "boolean"},
                        "origin": {"$ref": "#/components/schemas/SessionOrigin"}
                    }
                },
                "AnnotationGroup": {""",
            """                        "starred": {"type": "boolean"}
                    }
                },
                "AnnotationGroup": {""",
        ),
        "every_field_a_response_sends_is_described_in_the_spec",
    ),
    (
        "a labelled metric ships no vocabulary",
        text_edit(
            METRICS,
            """    for result in ["ok", "error"] {
        OCR_REQUESTS.with_label_values(&[result]).inc_by(0);
    }""",
            "",
        ),
        "labelled_metrics_export_their_whole_vocabulary_before_any_event",
    ),
    (
        "a closed websocket is never given back to the gauge",
        text_edit(
            WSI,
            """    info!(session_id = %id, "ws disconnected");
    crate::metrics::WS_CONNECTIONS_ACTIVE.dec();""",
            """    info!(session_id = %id, "ws disconnected");""",
        ),
        "the_connection_gauge_counts_up_and_back_down_to_zero",
    ),
    (
        "an acknowledgement always sends its (empty) arrays",
        text_edit(
            WSI,
            """        #[serde(skip_serializing_if = "Vec::is_empty", default)]
        consumed_annotations: Vec<i32>,""",
            """        #[serde(default)]
        consumed_annotations: Vec<i32>,""",
        ),
        "an_empty_acknowledgement_does_not_send_empty_arrays",
    ),
    (
        "the remove request goes back to only cancelling",
        text_edit(
            APPI,
            """    Query(params): Query<DeleteParams>,
) -> Response {
    if params.remove {""",
            """    Query(params): Query<DeleteParams>,
) -> Response {
    if false && params.remove {""",
        ),
        "remove_forgets_the_session_and_deletes_its_files",
    ),
    (
        "a multipart verdict is dropped again",
        text_edit(
            APPI,
            """            "verdict" => {""",
            """            "verdict_ignored" => {""",
        ),
        "a_submission_with_a_broken_field_is_told_which_field",
    ),
    (
        "the app forgets to register its metrics",
        text_edit(
            APPI,
            """    crate::metrics::init();
""",
            "",
        ),
        "metrics_are_served_in_the_format_prometheus_scrapes",
    ),
    (
        "an out-of-range annotation index is accepted",
        text_edit(
            APPI,
            """            else {
                return bad_request("consumed annotation index is out of range".into());
            };""",
            """            else {
                continue;
            };""",
        ),
        "an_acknowledgement_must_name_the_feedback_it_consumes",
    ),
    (
        "an empty canvas is still sent to be read",
        text_edit(
            OCRI,
            """        if strokes.is_empty() {
            return Ok(String::new());
        }""",
            """        if strokes.is_empty() {
            return self.recognize_image(&[]).await;
        }""",
        ),
        "handwriting_with_nothing_drawn_makes_no_request",
    ),
    (
        "the cloud timeout defaults to zero",
        text_edit(
            PROVIDERSI,
            "    bounded_seconds(value, 15, 120)",
            "    bounded_seconds(value, 0, 120)",
        ),
        "the_cloud_timeout_stays_inside_a_usable_range",
    ),
    (
        "the local runtime error loses its status",
        text_edit(
            PROVIDERSI,
            """            "local llama OCR returned HTTP {}",
            response.status().as_u16()""",
            """            "local llama OCR returned HTTP {}",
            response.status().is_success()""",
        ),
        "a_local_runtime_http_error_is_reported_with_its_status",
    ),
    (
                "pushed metric labels are not escaped",
        text_edit(
            CLII,
            r"""            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            other => out.push(other),""",
            """            other => out.push(other),""",
        ),
        "a_label_value_cannot_break_the_pushed_body",
    ),
    (
        "the ocr endpoint answers an empty transcription without an engine",
        text_edit(
            APPI,
            """            return (StatusCode::SERVICE_UNAVAILABLE, "OCR engine not available").into_response();""",
            """            return Json(serde_json::json!({ "text": "" })).into_response();""",
        ),
        "server_side_ocr_is_optional_and_says_so",
    ),
    (
                "the remove command forgets to ask for removal",
        text_edit(
            CLII,
            r"""    let mut url = format!("{server}/api/sessions/{id}?remove=true");""",
            r"""    let mut url = format!("{server}/api/sessions/{id}");""",
        ),
        "cli_remove_takes_the_review_away_and_cancel_keeps_it",
    ),
    (
        "the fallback for a missing leading newline deletes the code",
        text_edit(
            DIAGRAMS,
            ".unwrap_or(inner);",
            '.unwrap_or("");',
        ),
        "a_block_without_a_trailing_newline_still_counts_its_last_line",
    ),
    (
        "a delivered webhook is not counted",
        text_edit(
            APPI,
            """            Ok(resp) if resp.status().is_success() => {
                crate::metrics::WEBHOOK_TOTAL
                    .with_label_values(&["ok"])
                    .inc();""",
            """            Ok(resp) if resp.status().is_success() => {
                let _ = resp;""",
        ),
        "the_webhook_metric_counts_what_actually_happened",
    ),
    (
        "the cli stops reporting its commands",
        text_edit(
            CLII,
            """    let Ok(gw) = std::env::var("EINK_PUSHGATEWAY_URL") else {
        return;
    };""",
            """    let Ok(gw) = std::env::var("EINK_PUSHGATEWAY_URL") else {
        return;
    };
    return;""",
        ),
        "the_cli_pushes_its_own_command_metrics",
    ),
    (
        "an annotation path may point outside the session",
        text_edit(
            APPI,
            "    candidate.starts_with(&root).then_some(candidate)",
            "    Some(candidate)",
        ),
        "an_annotation_index_cannot_escape_the_session_directory",
    ),
    (
        "a reconnecting client is not told how the review ended",
        text_edit(
            WSI,
            "        SessionStatus::Active => None,",
            "        SessionStatus::Active | SessionStatus::Submitted => None,",
        ),
        "a_finished_session_says_how_it_ended_instead_of_going_quiet",
    ),
    (
        "a finished session still replays its old update",
        text_edit(
            WSI,
            '        .filter(|s| s.status == crate::session::SessionStatus::Active)',
            "        .filter(|_| true)",
        ),
        "a_finished_session_has_no_snapshot_to_replay",
    ),
    (
        "a silent client is never dropped",
        text_edit(
            WSI,
            "if last_seen.elapsed() > std::time::Duration::from_secs(heartbeat * 3) {",
            "if false {",
        ),
        "a_client_that_stops_answering_is_dropped",
    ),
    (
        "an occupied port panics again",
        text_edit(
            MAINI,
            """        Err(error) => {
            eprintln!("eink-serve: cannot listen on {addr}: {error}");
            tracing::error!(%addr, %error, "cannot listen");
            std::process::exit(1);
        }
    };""",
            """        Err(error) => panic!("cannot listen on {addr}: {error}"),
    };""",
        ),
        "an_occupied_port_is_reported_in_words",
    ),
    (
        "removing a live review is allowed again",
        text_edit(
            APPI,
            """            if live {
                return (
                    StatusCode::CONFLICT,""",
            """            if false && live {
                return (
                    StatusCode::CONFLICT,""",
        ),
        "remove_forgets_the_session_and_deletes_its_files",
    ),
    (
        "a zero OCR bound is accepted",
        text_edit(
            PROVIDERSI,
            "        .filter(|v| (1..=max).contains(v))",
            "        .filter(|v| (0..=max).contains(v))",
        ),
        "a_bound_is_never_zero_and_never_unbounded",
    ),
    (
        "the interactive OCR bound is as long as the batch one",
        text_edit(
            OCRI,
            "providers::bounded_seconds(interactive, 20, 120)",
            "providers::bounded_seconds(interactive, 120, 120)",
        ),
        "the_interactive_bound_is_shorter_than_the_batch_one_by_default",
    ),
    (
        "the health check claims a writable directory it never tried",
        text_edit(
            APPI,
            """    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }""",
            """    let _ = std::fs::File::create(&probe);
    true""",
        ),
        "health_says_what_is_actually_wrong",
    ),
    (
        "a stroke far from every paragraph is bound anyway",
        text_edit(
            STROKEBUFFER,
            "        if (distToElement(cx, cy, nearest) > maxBindDistance) {",
            "        if (false) {",
        ),
        "gradle:StrokeBufferTest",
    ),
    (
        "the build filter loses the metrics reference",
        text_edit(
            FLAKE,
            """              (pkgs.lib.hasInfix "METRICS.md" path);""",
            """              false;""",
        ),
        "the_allowlist_matches_the_filter_in_the_flake",
    ),
    (
        "a page ignores its own cursor and repeats itself",
        text_edit(
            APPI,
            """        if let Some((starred, micros, id)) = &after {""",
            """        if let Some((starred, micros, id)) = None::<&(bool, i64, String)> {""",
        ),
        "the_session_list_pages_without_repeating_or_dropping_a_session",
    ),
    (
        "a page size of zero is served",
        text_edit(
            APPI,
            """    if params.limit == Some(0) {
        return (StatusCode::BAD_REQUEST, "limit must be at least 1").into_response();
    }""",
            "",
        ),
        "a_page_size_of_zero_or_a_bad_cursor_is_refused",
    ),
    (
        "the sort loses its tiebreaker",
        text_edit(
            APPI,
            "    (session.starred, session.updated_at, session.id.clone())",
            "    (session.starred, session.updated_at, String::new())",
        ),
        "the_session_list_pages_without_repeating_or_dropping_a_session",
    ),
    (
        "the response stops waiting for the write",
        text_edit(
            APPI,
            "        && let Err(error) = state.flush_writes().await",
            "        && let Err(error) = Ok::<(), String>(())",
        ),
        "a_response_means_the_change_is_on_disk",
    ),
    (
        "the write queue is bypassed and the caller waits on the disk",
        text_edit(
            SESSIONI,
            """                Some(writer) => {
                    if writer""",
            """                Some(_writer) => {
                    if false""",
        ),
        "a_queued_write_does_not_hold_the_caller",
    ),
    (
        "the submit response is opened and never closed",
        text_edit(
            SUBMISSION,
            """            client.newCall(Request.Builder().url(url).post(builder.build()).build())
                .execute()
                .use { response ->""",
            """            client.newCall(Request.Builder().url(url).post(builder.build()).build())
                .execute()
                .let { response ->""",
        ),
        "gradle:SubmissionManagerTest",
    ),
    (
        "every HTTP code counts as a successful submission",
        text_edit(
            SUBMISSION,
            "    val isSuccessful get() = this is Http && code in 200..299",
            "    val isSuccessful get() = this is Http",
        ),
        "gradle:SubmissionManagerTest",
    ),
    (
        "a partial page counts as a complete one",
        text_edit(
            SESSIONVM,
            """    complete -> ListOutcome.UseFetchedAndCache
    else -> ListOutcome.UseFetchedOnly""",
            """    complete -> ListOutcome.UseFetchedOnly
    else -> ListOutcome.UseFetchedAndCache""",
        ),
        "gradle:SessionPagingTest",
    ),
    (
        "a failed refresh blanks the list",
        text_edit(
            SESSIONVM,
            """    fetchedCount == 0 -> ListOutcome.UseCached
""",
            "",
        ),
        "gradle:SessionPagingTest",
    ),
    (
        "the ink mapping uses the browser zoom factor instead of the view",
        text_edit(
            STROKEBUFFER,
            """    val pxPerCss = viewWidthPx / report.widthCss""",
            """    val pxPerCss = report.scale""",
        ),
        "gradle:ViewportTransformTest",
    ),
    (
        "the ink mapping forgets the pan",
        text_edit(
            STROKEBUFFER,
            """        scrollX = report.pageLeftCss * pxPerCss,
        scrollY = report.pageTopCss * pxPerCss,""",
            """        scrollX = 0f,
        scrollY = 0f,""",
        ),
        "gradle:ViewportTransformTest",
    ),
    (
        "a mode change keeps the finger the toolbar tap left behind",
        text_edit(
            PENOVERLAY,
            """        // proof that no finger is on the page.
        fingerDown = false""",
            """        // proof that no finger is on the page.""",
        ),
        "gradle:PenOverlayTest",
    ),
    (
        "a cancelled drag is thrown away instead of committed",
        text_edit(
            PENOVERLAY,
            """                        when {
                            dragClusterIdx != null -> commitClusterOffset(dragClusterIdx!!)
                            dragGroupId != null -> commitGroupOffset(dragGroupId!!)
                        }
                        dragLabel = null; dragGroupId = null; dragClusterIdx = null""",
            """                        if (event.actionMasked == MotionEvent.ACTION_UP) {
                            when {
                                dragClusterIdx != null -> commitClusterOffset(dragClusterIdx!!)
                                dragGroupId != null -> commitGroupOffset(dragGroupId!!)
                            }
                        }
                        dragLabel = null; dragGroupId = null; dragClusterIdx = null""",
        ),
        "gradle:PenOverlayTest",
    ),
    (
        "a moved group leaves its OCR label behind",
        text_edit(
            PENOVERLAY,
            """            for (i in ocrResults.indices) {
                val result = ocrResults[i]
                if (result.strokeIndices.any { it in group.strokeIndices }) {
                    ocrResults[i] = result.copy(
                        docX = result.docX + dx,
                        docY = result.docY + dy,
                        minDocY = result.minDocY + dy,
                    )
                }
            }
""",
            "",
        ),
        "gradle:PenOverlayTest",
    ),
    (
        "a resize mid-drag drops the drag instead of committing it",
        text_edit(
            PENOVERLAY,
            """        when {
            dragClusterIdx != null -> commitClusterOffset(dragClusterIdx!!)
            dragGroupId != null -> commitGroupOffset(dragGroupId!!)
        }
        dragLabel = null""",
            """        dragLabel = null""",
        ),
        "gradle:PenOverlayTest",
    ),
]


def main() -> int:
    only = sys.argv[1:]
    cases = [
        case
        for case in CASES
        if not only or any(word in case[0] or word in case[2] for word in only)
    ]
    if only and not cases:
        print(f"no mutation matches {only}")
        return 1
    missed = []
    for label, (apply, revert), test in cases:
        apply()
        try:
            caught = not suite_is_green(test, target_for(test))
        finally:
            revert()
        print(f"  {'caught' if caught else 'MISSED':6} {test:52} <- {label}")
        if not caught:
            missed.append((label, test))

    print()
    if missed:
        print(f"{len(missed)} of {len(cases)} mutations slipped through:")
        for label, test in missed:
            print(f"  - {label} (expected '{test}' to fail)")
        return 1
    print(f"all {len(cases)} mutations were caught")
    if only:
        return 0

    # No target filter: every test target, doc tests included. A gate that checks less
    # than the mutations touch is how a swapped pair of commands once got through.
    if not suite_is_green():
        print("FAIL: the suite is not green after reverting every mutation")
        return 1
    print("suite green again after reverting every mutation")
    return 0


if __name__ == "__main__":
    sys.exit(main())
