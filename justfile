# E-Ink Bridge task runner
# Run `just` to see all recipes, `just <recipe>` to run one.

server_dir := "server"
state_dir := env("XDG_DATA_HOME", env("HOME", "/tmp") + "/.local/share") + "/eink-bridge"
config_dir := env("XDG_CONFIG_HOME", env("HOME", "/tmp") + "/.config") + "/eink-bridge"

# list all recipes
default:
    @just --list --unsorted

# ─── Testing ───────────────────────────────────────────────────────────

# run all server tests (unit + integration)
test:
    cd {{server_dir}} && cargo test

# render unit tests only (~0.02s)
test-render:
    cd {{server_dir}} && cargo test render::tests

# run a single test by name
test-one name:
    cd {{server_dir}} && cargo test {{name}}

# HTTP integration tests
test-http:
    cd {{server_dir}} && cargo test --test health_test --test session_lifecycle_test --test long_poll_test --test render_validation_test

# E2E tests (spawns real server + mock device)
test-e2e:
    cd {{server_dir}} && cargo test --test e2e_test --test cli_integration_test

# run tests with output visible (for debugging)
test-verbose *args='':
    cd {{server_dir}} && cargo test {{args}} -- --nocapture

# AI-native eval: render goldens + contract goldens
eval:
    cd {{server_dir}} && cargo test --test eval_render_golden_test --test eval_contract_test -- --nocapture

# refresh eval goldens intentionally
eval-update:
    cd {{server_dir}} && UPDATE_GOLDENS=1 cargo test --test eval_render_golden_test --test eval_contract_test -- --nocapture

# ─── Coverage ─────────────────────────────────────────────────────────

# run tests with coverage summary (text report)
coverage:
    cd {{server_dir}} && cargo llvm-cov --text

# run tests with detailed per-file coverage
coverage-detail:
    cd {{server_dir}} && cargo llvm-cov --text --show-instantiations

# generate HTML coverage report and open it
coverage-html:
    cd {{server_dir}} && cargo llvm-cov --html
    @echo "report: {{server_dir}}/target/llvm-cov/html/index.html"

# show only uncovered regions (lines not hit by any test)
coverage-uncovered:
    cd {{server_dir}} && cargo llvm-cov --text --show-missing-lines

# ─── Lint & Format ────────────────────────────────────────────────────

# format + clippy
lint:
    cd {{server_dir}} && cargo fmt && cargo clippy

# check formatting without modifying
check-fmt:
    cd {{server_dir}} && cargo fmt -- --check

# clippy only
clippy:
    cd {{server_dir}} && cargo clippy

# format only
fmt:
    cd {{server_dir}} && cargo fmt

# ─── Build ─────────────────────────────────────────────────────────────

# cargo dev build
build:
    cd {{server_dir}} && cargo build

# cargo release build
build-release:
    cd {{server_dir}} && cargo build --release

# nix build (runs tests too)
nix-build:
    nix build

# ─── Deploy ────────────────────────────────────────────────────────────

# build + restart systemd service
deploy: nix-build
    systemctl --user restart eink-serve
    @echo "restarted eink-serve"
    @systemctl --user is-active eink-serve

# show service status
status:
    systemctl --user status eink-serve

# follow server logs
logs:
    journalctl --user -u eink-serve -f

# show recent server logs
logs-recent n='50':
    journalctl --user -u eink-serve -n {{n}}

# ─── Android ───────────────────────────────────────────────────────────

# run android unit tests
test-android:
    cd android && ./gradlew testDebugUnitTest

# run android unit tests with JaCoCo coverage report
coverage-android:
    cd android && ./gradlew testDebugUnitTest jacocoTestReport
    @echo "report: android/app/build/reports/jacoco/jacocoTestReport/html/index.html"

# regenerate golden PNG snapshots for RenderGoldenTest
golden-android:
    cd android && UPDATE_GOLDENS=1 ./gradlew testDebugUnitTest --tests "*.RenderGoldenTest*"

# E2E annotation round-trip (requires built eink-serve binary)
test-android-e2e:
    cd android && ANTHROPIC_API_KEY=$(cat ~/.config/anthropic/key 2>/dev/null || true) \
      ./gradlew testDebugUnitTest --tests "*AnnotationE2ETest*"

# take screenshot from connected device
adb-screenshot path='screen.png':
    nix-shell -p android-tools --run "adb exec-out screencap -p > {{path}} && echo 'saved {{path}}'"

# dump UI hierarchy to XML
adb-ui-dump path='ui.xml':
    nix-shell -p android-tools --run "adb shell uiautomator dump /sdcard/ui.xml && adb pull /sdcard/ui.xml {{path}}"

# tap by screen coordinates
adb-tap x y:
    nix-shell -p android-tools --run "adb shell input tap {{x}} {{y}}"

# stream logcat filtered to app tags
adb-log:
    nix-shell -p android-tools --run "adb logcat -s PenOverlay:D MainActivity:D"

# build android debug APK
apk:
    cd android && ./gradlew assembleDebug

# build + install APK on connected device
apk-install: apk
    nix-shell -p android-tools --run "adb install -r android/app/build/outputs/apk/debug/app-debug.apk"

# set boox hidden_api_policy (required once for pen SDK)
boox-setup:
    nix-shell -p android-tools --run "adb shell settings put global hidden_api_policy 1"

# ─── Dev ───────────────────────────────────────────────────────────────

# run server locally with debug logging
run:
    cd {{server_dir}} && RUST_LOG=debug cargo run --bin eink-serve

# push a file to the tablet (blocking)
push file:
    cd {{server_dir}} && cargo run --bin eink-review -- push {{file}}

# push stdin to the tablet
push-stdin:
    cd {{server_dir}} && cargo run --bin eink-review -- push -

# list active sessions
sessions:
    cd {{server_dir}} && cargo run --bin eink-review -- list

# run mock device (auto-submits "LGTM")
mock-device:
    cd {{server_dir}} && cargo run --bin eink-mock-device -- --notes "LGTM" --once

# manual E2E: server + mock device + push a test doc
manual-e2e file='README.md':
    #!/usr/bin/env bash
    set -euo pipefail
    cd {{server_dir}}
    cargo build
    cleanup() { kill $SERVER_PID 2>/dev/null || true; }
    trap cleanup EXIT
    RUST_LOG=info cargo run --bin eink-serve &
    SERVER_PID=$!
    sleep 1
    cargo run --bin eink-mock-device -- --notes "LGTM" --once &
    sleep 0.5
    cargo run --bin eink-review -- push ../{{file}}

# ─── CI / pre-commit checks ───────────────────────────────────────────

# full check: fmt + clippy + test + eval + nix build
ci: check-fmt clippy test eval nix-build
    @echo "all checks passed"

# quick pre-commit: fmt check + clippy + fast tests
pre-commit: check-fmt clippy test
    @echo "ready to commit"

# ─── Debug ─────────────────────────────────────────────────────────────

# run server with trace-level logging (max verbosity)
run-trace:
    cd {{server_dir}} && RUST_LOG=trace cargo run --bin eink-serve

# show config file location and contents
show-config:
    @echo "{{config_dir}}/config.toml"
    @cat {{config_dir}}/config.toml 2>/dev/null || echo "(no config file, using defaults)"

# show state directory contents
show-state:
    @ls -la {{state_dir}}/ 2>/dev/null || echo "(no state dir at {{state_dir}})"

# show active sessions in state dir
show-sessions:
    @ls -la {{state_dir}}/sessions/ 2>/dev/null || echo "(no sessions)"

# inspect a session's persisted JSON
inspect-session id:
    @cat {{state_dir}}/sessions/{{id}}/session.json 2>/dev/null | python3 -m json.tool || echo "session {{id}} not found on disk"

# hit the health endpoint
health host='localhost:3333':
    @curl -sf http://{{host}}/api/health && echo "" || echo "UNREACHABLE"

# list sessions via API
api-sessions host='localhost:3333':
    curl -s http://{{host}}/api/sessions | python3 -m json.tool

# get session details via API
api-session id host='localhost:3333':
    curl -s http://{{host}}/api/sessions/{{id}} | python3 -m json.tool

# create a test session via API and print its URL
api-test-session host='localhost:3333':
    #!/usr/bin/env bash
    set -euo pipefail
    RESP=$(curl -sf -X POST http://{{host}}/api/sessions?title=debug-test \
      -d '# Debug Test Session\n\nHello from `just api-test-session`')
    echo "$RESP" | python3 -m json.tool
    ID=$(echo "$RESP" | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
    echo "View: http://{{host}}/session/$ID"

# check server connectivity and basic health
doctor host='localhost:3333':
    #!/usr/bin/env bash
    set -euo pipefail
    echo "=== eink-bridge doctor ==="
    echo ""
    echo "Service:"
    systemctl --user is-active eink-serve 2>/dev/null && echo "  eink-serve: running" || echo "  eink-serve: NOT running"
    echo ""
    echo "Health:"
    curl -sf http://{{host}}/api/health > /dev/null 2>&1 && echo "  http://{{host}}: ok" || echo "  http://{{host}}: UNREACHABLE"
    echo ""
    echo "Config:"
    CFG="{{config_dir}}/config.toml"
    [ -f "$CFG" ] && echo "  $CFG: present" || echo "  $CFG: (defaults)"
    echo ""
    echo "State dir:"
    STATE="{{state_dir}}"
    if [ -d "$STATE/sessions" ]; then
        TOTAL=$(ls "$STATE/sessions" 2>/dev/null | wc -l)
        echo "  $STATE: $TOTAL sessions on disk"
    else
        echo "  $STATE: (empty)"
    fi
    echo ""
    echo "Sessions (API):"
    SESSIONS=$(curl -sf http://{{host}}/api/sessions 2>/dev/null)
    if [ $? -eq 0 ]; then
        ACTIVE=$(echo "$SESSIONS" | python3 -c "import sys,json; print(sum(1 for s in json.load(sys.stdin) if s['status']=='Active'))" 2>/dev/null || echo "?")
        SUBMITTED=$(echo "$SESSIONS" | python3 -c "import sys,json; print(sum(1 for s in json.load(sys.stdin) if s['status']=='Submitted'))" 2>/dev/null || echo "?")
        echo "  active: $ACTIVE, submitted: $SUBMITTED"
    else
        echo "  (server unreachable)"
    fi
    echo ""
    echo "Android (adb):"
    ADB=$(which adb 2>/dev/null || echo "")
    if [ -n "$ADB" ]; then
        $ADB devices 2>/dev/null | grep -q device && echo "  device connected" || echo "  no device"
    else
        echo "  adb not in PATH"
    fi

# watch for changes and re-run render tests
watch-render:
    cd {{server_dir}} && cargo watch -c -x 'test render::tests'

# watch for changes and re-run all tests
watch:
    cd {{server_dir}} && cargo watch -c -x test

# ─── Cleanup ───────────────────────────────────────────────────────────

# remove build artifacts
clean:
    cd {{server_dir}} && cargo clean

# remove nix result symlink
clean-result:
    rm -f result
