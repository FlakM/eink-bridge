---
name: coverage
description: Run test coverage analysis across Rust server, Kotlin Android app, and CLI. Identifies uncovered code and suggests where to add tests.
---

# Coverage Analysis

Run coverage tools for the project's components and analyze gaps.

## Usage

```
/coverage [component]
```

- No argument: run all components and produce a combined summary.
- `server` or `rust`: Rust server + CLI only (`cargo llvm-cov`)
- `android` or `kotlin`: Android/Kotlin only (JaCoCo)
- A specific module name (e.g. `config`, `session`): focused coverage on that module.

## Rust Server + CLI

1. Run from the `server/` directory:

```bash
cd server && just coverage
# or directly:
cd server && nix develop .. --command cargo llvm-cov --text
```

2. Parse the summary table at the bottom:

```
Filename          Lines  Missed  Cover
app.rs              252       7  97.2%
config.rs            76      17  77.6%
...
TOTAL              1425     101  92.9%
```

3. For files below 90% line coverage, run the detailed view to find specific uncovered lines:

```bash
cd server && nix develop .. --command cargo llvm-cov --text --show-missing-lines
```

## Android / Kotlin

1. Run from project root:

```bash
just coverage-android
# or directly:
cd android && nix-shell shell.nix --run './gradlew testDebugUnitTest jacocoTestReport'
```

2. The HTML report is at `android/app/build/reports/jacoco/jacocoTestReport/html/index.html`.
   The XML report (machine-parseable) is at the same path with `.xml` extension.

3. Read the XML or HTML to extract per-class coverage. Key classes to check:
   - `StrokeBuffer` — pure data structure, should be near 100%
   - `SessionAdapter` (statusIcon, formatSessionTime) — pure utility functions
   - `PenOverlay` (rawDrawingAction) — pure decision function
   - `MainActivity` — Android-coupled, low coverage expected (needs instrumentation tests)

## Reporting

Present findings as:

**Coverage Summary**

| Component | File | Line Coverage | Key Gaps |
|-----------|------|-------------|----------|
| server | app.rs | 97% | - |
| server | config.rs | 78% | `load()` error paths |
| android | StrokeBuffer | 95% | - |
| android | MainActivity | 0% | Android-coupled, needs instrumentation |

**Uncovered Areas Needing Tests**

For each gap, explain:
- What code path is uncovered
- Why it matters (error handling? edge case? happy path?)
- A concrete test suggestion

## Important

- `main.rs` and `mock_device.rs` are binary entrypoints — low coverage is expected and not worth flagging.
- `MainActivity.kt` requires Android instrumentation tests (not JUnit) — flag this as a known gap but don't suggest unit tests for Android framework code.
- Focus on testable pure logic that has gaps: error paths, boundary conditions, public API surface.
- Prioritize: untested error paths > untested happy paths > display/formatting code.
