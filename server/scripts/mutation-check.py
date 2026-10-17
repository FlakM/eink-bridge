#!/usr/bin/env python3
"""Prove the render invariants bite: reintroduce each bug and watch a test go red.

The tests in `src/render.rs` exist because of specific regressions -- a clipped
annotation outline, a canvas that went dark, a steps block that ate the pen's taps. A
green suite only shows the tests pass; it does not show they would notice the bug
coming back. This script applies each defect on its own, asserts the named test fails,
reverts, and finally asserts the suite is green again.

    just test-mutations
"""
from __future__ import annotations

import pathlib
import plistlib
import re
import subprocess
import sys

SERVER = pathlib.Path(__file__).resolve().parent.parent
RENDER = SERVER / "src/render.rs"
DIAGRAMS = SERVER / "src/render/diagrams.rs"
CHUNKS = SERVER / "src/render/chunks.rs"
REANCHOR = SERVER / "src/reanchor.rs"
CLII = SERVER / "src/cli.rs"
THEME = SERVER / "assets/eink-theme.tmTheme"


# Tests in a binary crate are invisible to `--lib`; they run against the bin target.
BIN_TARGETS = {
    "a_review_exit_code_is_the_contract_a_pipeline_gates_on": "--bin eink-review",
    "long_element_text_is_elided_without_panicking_on_a_character_boundary": "--bin eink-review",
    "the_observer_url_follows_the_scheme_of_the_server": "--bin eink-review",
    "a_session_row_shows_the_star_the_status_and_where_it_came_from": "--bin eink-review",
    "a_submitted_review_renders_notes_anchors_and_images": "--bin eink-review",
}


def target_for(test: str) -> str:
    return BIN_TARGETS.get(test, "--lib")


def suite_is_green(test: str = "", target: str = "--lib") -> bool:
    proc = subprocess.run(
        ["cargo", "test", *target.split(), "--quiet", test],
        cwd=SERVER,
        capture_output=True,
        text=True,
    )
    return proc.returncode == 0


def locate(body: str, needle: str) -> tuple[int, int] | None:
    """Find `needle`, ignoring how the formatter wrapped it."""
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
    saved: dict[str, str] = {}

    def apply():
        body = path.read_text()
        found = locate(body, old)
        assert found, f"anchor missing in {path}: {old[:70]!r}"
        start, end = found
        saved["text"] = body[start:end]
        path.write_text(body[:start] + new + body[end:])

    def revert():
        body = path.read_text()
        found = locate(body, new)
        assert found, f"mutation not present in {path}: {new[:70]!r}"
        start, end = found
        path.write_text(body[:start] + saved["text"] + body[end:])

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
            RENDER,
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
            RENDER,
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
            RENDER,
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
            RENDER,
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
            RENDER,
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
            RENDER,
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
]


def main() -> int:
    missed = []
    for label, (apply, revert), test in CASES:
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
        print(f"{len(missed)} of {len(CASES)} mutations slipped through:")
        for label, test in missed:
            print(f"  - {label} (expected '{test}' to fail)")
        return 1
    print(f"all {len(CASES)} mutations were caught")

    if not suite_is_green():
        print("FAIL: the suite is not green after reverting every mutation")
        return 1
    print("suite green again after reverting every mutation")
    return 0


if __name__ == "__main__":
    sys.exit(main())
