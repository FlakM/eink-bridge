#!/usr/bin/env bash
# Run the instrumented suite on an emulator, against a server of its own.
#
# Everything here is disposable: the emulator is headless and killed on exit, and the
# server writes to a temporary XDG directory, so the reader's own reviews are never
# touched and a failed run leaves nothing behind.
#
#   ./scripts/emulator-test.sh            # whole suite
#   ./scripts/emulator-test.sh AppFlowTest#aSubmittedNoteReachesTheServer
#
# Requires KVM. On NixOS the emulator comes from the Android SDK in the dev shell; on CI
# the same, and /dev/kvm is present on GitHub's Linux runners.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ANDROID_DIR="$(cd "$HERE/.." && pwd)"
REPO="$(cd "$ANDROID_DIR/.." && pwd)"

SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
if [ -z "$SDK" ] || [ ! -d "$SDK" ]; then
    echo "error: ANDROID_HOME is not set; run this inside the dev shell (direnv, or nix develop)" >&2
    exit 1
fi

if [ ! -e /dev/kvm ]; then
    echo "error: /dev/kvm is missing, so the emulator would crawl. Add the user to the kvm group." >&2
    exit 1
fi

AVD_NAME="${AVD_NAME:-eink-test}"
PORT="${EINK_TEST_PORT:-4123}"
FILTER="${1:-}"
TMP="$(mktemp -d)"
ADB="$SDK/platform-tools/adb"
EMULATOR_PID=""
SERVER_PID=""
SERIAL=""

cleanup() {
    status=$?
    [ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2>/dev/null || true
    [ -n "$EMULATOR_PID" ] && kill "$EMULATOR_PID" 2>/dev/null || true
    # adb keeps the emulator registered for a moment after the process dies.
    [ -n "$SERIAL" ] && "$ADB" -s "$SERIAL" emu kill 2>/dev/null || true
    rm -rf "$TMP"
    exit $status
}
trap cleanup EXIT INT TERM

# ── the server, on a state directory of its own ──────────────────────────────
mkdir -p "$TMP/config/eink-bridge" "$TMP/data"
cat > "$TMP/config/eink-bridge/config.toml" <<EOF
[server]
host = "127.0.0.1"
port = $PORT
state_dir = "$TMP/data/state"
EOF

SERVE_BIN="${EINK_SERVE_BIN:-$REPO/server/target/debug/eink-serve}"
if [ ! -x "$SERVE_BIN" ]; then
    echo "building the server once…"
    (cd "$REPO/server" && cargo build --bin eink-serve)
fi

echo "starting the server on 127.0.0.1:$PORT with state in $TMP/data"
XDG_CONFIG_HOME="$TMP/config" XDG_DATA_HOME="$TMP/data" RUST_LOG=warn \
    "$SERVE_BIN" >"$TMP/server.log" 2>&1 &
SERVER_PID=$!

for _ in $(seq 1 60); do
    if curl -sf "http://127.0.0.1:$PORT/api/health" >/dev/null 2>&1; then break; fi
    sleep 0.5
done
if ! curl -sf "http://127.0.0.1:$PORT/api/health" >/dev/null 2>&1; then
    echo "error: the server never answered; its log:" >&2
    tail -20 "$TMP/server.log" >&2
    exit 1
fi

# ── the emulator ─────────────────────────────────────────────────────────────
# Every adb call below names the device: the tablet is often plugged in at the same time,
# and an unqualified `adb shell` is "more than one device".
SERIAL="$("$ADB" devices | awk '/^emulator-/{print $1; exit}')"
if [ -z "$SERIAL" ]; then
    AVDMANAGER="$(command -v avdmanager || find "$SDK/cmdline-tools" -name avdmanager -type f | head -1)"
    if [ -z "$AVDMANAGER" ]; then
        echo "error: avdmanager is not in this SDK" >&2
        exit 1
    fi
    if ! "$AVDMANAGER" list avd 2>/dev/null | grep -q "Name: $AVD_NAME"; then
        echo "creating the $AVD_NAME image (once)"
        echo no | "$AVDMANAGER" create avd \
            -n "$AVD_NAME" -k "system-images;android-34;google_apis;x86_64" \
            -d "Nexus 10" --force >/dev/null
    fi
    echo "booting the emulator headless"
    "$SDK/emulator/emulator" -avd "$AVD_NAME" -no-window -no-audio -no-snapshot \
        -no-boot-anim -gpu swiftshader_indirect >"$TMP/emulator.log" 2>&1 &
    EMULATOR_PID=$!

    for _ in $(seq 1 60); do
        SERIAL="$("$ADB" devices | awk '/^emulator-/{print $1; exit}')"
        [ -n "$SERIAL" ] && break
        sleep 1
    done
    if [ -z "$SERIAL" ]; then
        echo "error: the emulator never registered with adb; its log:" >&2
        tail -20 "$TMP/emulator.log" >&2
        exit 1
    fi
fi

echo "waiting for $SERIAL to boot"
for _ in $(seq 1 120); do
    booted="$("$ADB" -s "$SERIAL" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')"
    [ "$booted" = "1" ] && break
    sleep 2
done
if [ "$("$ADB" -s "$SERIAL" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" != "1" ]; then
    echo "error: $SERIAL never finished booting; its log:" >&2
    tail -20 "$TMP/emulator.log" 2>/dev/null >&2
    exit 1
fi

echo "running the instrumented suite on $SERIAL"

# `10.0.2.2` is how the emulator reaches the host's loopback.
cd "$ANDROID_DIR"
ANDROID_SERIAL="$SERIAL" ./gradlew --console=plain connectedDebugAndroidTest \
    "-Pandroid.testInstrumentationRunnerArguments.serverUrl=http://10.0.2.2:$PORT" \
    ${FILTER:+-Pandroid.testInstrumentationRunnerArguments.class="com.flakm.einkbridge.$FILTER"}
