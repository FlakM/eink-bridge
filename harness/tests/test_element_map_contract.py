"""Every page function the tablet calls must exist in the page.

`element-map.js` is a contract between two languages: Kotlin calls `window.__eink…` by
name and the JavaScript defines it. Nothing checks the join, so a rename on either side —
or a call written before the function was — fails silently at runtime. That is exactly how
tapping a block to speak at it stopped working: `findNearestElement` called
`__einkFindNearestElement`, which the page never defined, and every tap answered
"nothing to talk about there".
"""
import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
ELEMENT_MAP = ROOT / "server/assets/element-map.js"
ANDROID_SRC = ROOT / "android/app/src/main/java"

DEFINITION = re.compile(r"window\.(__eink[A-Za-z]+)\s*=")
REFERENCE = re.compile(r"(__eink[A-Za-z]+)")


def defined_functions():
    return set(DEFINITION.findall(ELEMENT_MAP.read_text()))


def referenced_functions():
    found = {}
    for path in ANDROID_SRC.rglob("*.kt"):
        for name in REFERENCE.findall(path.read_text()):
            found.setdefault(name, path)
    return found


class ElementMapContractTest(unittest.TestCase):
    def test_every_function_the_tablet_calls_exists_in_the_page(self):
        defined = defined_functions()
        missing = {n: p for n, p in referenced_functions().items() if n not in defined}
        self.assertEqual(
            missing, {},
            "the tablet calls page functions that do not exist: "
            + ", ".join(f"{n} ({p.name})" for n, p in sorted(missing.items())),
        )

    def test_the_page_does_not_grow_functions_nothing_calls(self):
        """A definition nothing calls is dead weight that reads like a feature.

        The reverse direction is worth holding too, but only for functions that look like
        the tablet's: the page also exposes helpers used by its own scripts (steps, links),
        which are listed here as intentional.
        """
        internal = {
            "__einkClearLinks",
            "__einkFinishLinkGroup",
            "__einkStepNext",
            "__einkStepReset",
            "__einkTapElement",
            "__einkUpdateStrokeLinks",
        }
        referenced = set(referenced_functions())
        orphans = defined_functions() - referenced - internal
        self.assertEqual(orphans, set(), f"defined but never called: {sorted(orphans)}")

    def test_the_nearest_element_answers_even_from_the_margin(self):
        """The search must not require a hit on the text: the margin is where notes go.

        This reads the function's shape rather than running it — the page needs a WebView —
        so it pins the two decisions that make the margin work: distance is measured to the
        element's box, and only the vertical gap is capped.
        """
        source = ELEMENT_MAP.read_text()
        start = source.index("window.__einkFindNearestElement")
        body = source[start:start + 1400]
        self.assertIn("Math.max(e.t - docY, 0, docY - e.b)", body, "vertical gap to the box")
        self.assertIn("Math.max(e.l - docX, 0, docX - e.r)", body, "horizontal gap to the box")
        self.assertIn("NEAREST_MAX_DY", body)
