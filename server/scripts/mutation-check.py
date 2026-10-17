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
import subprocess
import sys

SERVER = pathlib.Path(__file__).resolve().parent.parent
RENDER = SERVER / "src/render.rs"
DIAGRAMS = SERVER / "src/render/diagrams.rs"
THEME = SERVER / "assets/eink-theme.tmTheme"


def suite_is_green(test: str = "") -> bool:
    proc = subprocess.run(
        ["cargo", "test", "--lib", "--quiet", test],
        cwd=SERVER,
        capture_output=True,
        text=True,
    )
    return proc.returncode == 0


def text_edit(path: pathlib.Path, old: str, new: str):
    def apply():
        body = path.read_text()
        assert old in body, f"anchor missing in {path}: {old[:60]!r}"
        path.write_text(body.replace(old, new, 1))

    def revert():
        body = path.read_text()
        assert new in body, f"mutation not present in {path}"
        path.write_text(body.replace(new, old, 1))

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
]


def main() -> int:
    missed = []
    for label, (apply, revert), test in CASES:
        apply()
        try:
            caught = not suite_is_green(test)
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
