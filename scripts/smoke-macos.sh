#!/usr/bin/env bash
# Opens a built Valerie.app the way a person would and fails if it doesn't stay up: release.yml runs it on each macOS
# version before publishing, so a build that crashes at launch (like 0.1.486 on macOS 26) never reaches anyone.
#   scripts/smoke-macos.sh path/to/Valerie.app
# Uses its own empty folders, profile and port, so it never touches a real Valerie.
set -euo pipefail
APP="${1:?usage: $0 path/to/Valerie.app}"
BIN="$APP/Contents/MacOS/valerie"
PORT="${PORT:-7799}"
DIR="$(mktemp -d)"
export VALERIE_CONFIG_DIR="$DIR/config" VALERIE_DATA_DIR="$DIR/data" VALERIE_TEST_WINDOWS=1
mkdir -p "$VALERIE_CONFIG_DIR" "$VALERIE_DATA_DIR"
cat > "$VALERIE_CONFIG_DIR/config.yaml" <<YAML
mcp: { port: $PORT }
profiles: [{ name: Smoke }]
spaces: [{ name: Smoke, profile: Smoke }]
updates: { check: false }
welcome: false
YAML

fail() {
  echo "SMOKE TEST FAILED: $1" >&2
  echo "--- output" >&2; cat "$DIR/output.log" >&2 || true
  echo "--- log" >&2; cat "$VALERIE_DATA_DIR"/logs/*.log >&2 2>/dev/null || true
  kill "$PID" 2>/dev/null || true
  exit 1
}

"$BIN" >"$DIR/output.log" 2>&1 &
PID=$!
for _ in $(seq 1 60); do
  kill -0 "$PID" 2>/dev/null || fail "Valerie quit while starting"
  [ -f "$VALERIE_DATA_DIR/mcp-token" ] && curl -s -m 1 -o /dev/null "http://127.0.0.1:$PORT/" && break
  sleep 1
done
[ -f "$VALERIE_DATA_DIR/mcp-token" ] || fail "Valerie didn't finish starting in 60 seconds"

# Two tabs on one site share a web process (pages.share_process_by_site), the path that crashed on macOS 26.
for url in https://example.com/ https://example.com/?second https://example.org/; do
  "$BIN" call open_tab "{\"url\":\"$url\"}" >/dev/null || fail "couldn't open $url"
  kill -0 "$PID" 2>/dev/null || fail "Valerie quit after opening $url"
done
"$BIN" call run_command '{"command":{"cmd":"open_command_bar"}}' >/dev/null 2>&1 || true
sleep 20
kill -0 "$PID" 2>/dev/null || fail "Valerie quit within 20 seconds of opening tabs"
if grep -hE '\[(panic|crash)\]|couldn.t start|could not create webview' "$VALERIE_DATA_DIR"/logs/*.log; then
  fail "the log shows a panic, crash or a page that couldn't be made"
fi
kill "$PID"
wait "$PID" 2>/dev/null || true
rm -rf "$DIR"
echo "smoke test passed: $("$BIN" --version 2>/dev/null || echo Valerie) stayed up with tabs open"
