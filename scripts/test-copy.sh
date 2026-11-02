#!/usr/bin/env bash
# A test copy of Valerie that stays out of your way: its own empty folders and profile, and it never takes focus.
#   scripts/test-copy.sh up [windowless|window]    start it from target/debug (cargo build first). windowless (default): no
#                                                  window at all. window: real windows on a display that isn't the main one.
#                                                  A site's pop-ups are real windows in both modes and go to that display.
#                                                  With only one display, windows open on it, without taking focus.
#   scripts/test-copy.sh call <tool> '<json>'      call one of its MCP tools
#   scripts/test-copy.sh folder                    print its data folder (logs are in logs/)
#   scripts/test-copy.sh down                      stop it and delete its folders
# PORT=7791 and BIN=/path/to/valerie change the port (one copy per port) and the program.
set -euo pipefail
cd "$(dirname "$0")/.."

PORT="${PORT:-7790}"
BIN="${BIN:-$PWD/target/debug/valerie}"
DIR="${TMPDIR:-/tmp}/valerie-test-copy-$PORT"
export VALERIE_CONFIG_DIR="$DIR/config" VALERIE_DATA_DIR="$DIR/data"

is_up() { [ -f "$DIR/pid" ] && kill -0 "$(cat "$DIR/pid")" 2>/dev/null; }

case "${1:-}" in
  up)
    mode="${2:-windowless}"
    case "$mode" in windowless|window) ;; *) echo "unknown mode: $mode" >&2; exit 2 ;; esac
    [ -x "$BIN" ] || { echo "no program at $BIN: run cargo build" >&2; exit 1; }
    is_up && { echo "already up on port $PORT: run $0 down" >&2; exit 1; }
    rm -rf "$DIR"
    mkdir -p "$VALERIE_CONFIG_DIR" "$VALERIE_DATA_DIR"
    cat > "$VALERIE_CONFIG_DIR/config.yaml" <<YAML
mcp: { port: $PORT }
profiles: [{ name: Test }]
spaces: [{ name: Test, profile: Test }]
updates: { check: false }
welcome: false
YAML
    [ "$mode" = windowless ] && export VALERIE_STARTED_FOR_AGENT=1
    VALERIE_TEST_WINDOWS=1 nohup "$BIN" >"$DIR/output.log" 2>&1 &
    echo $! > "$DIR/pid"
    for _ in $(seq 1 30); do
      [ -f "$VALERIE_DATA_DIR/mcp-token" ] && curl -s -m 1 -o /dev/null "http://127.0.0.1:$PORT/" && { echo "up: $mode, port $PORT"; exit 0; }
      is_up || break
      sleep 1
    done
    echo "it didn't start; see $DIR/output.log" >&2
    exit 1
    ;;
  call)
    is_up || { echo "not up: run $0 up" >&2; exit 1; }
    "$BIN" call "$2" "${3:-{\}}"
    ;;
  folder) echo "$VALERIE_DATA_DIR" ;;
  down)
    if is_up; then
      kill "$(cat "$DIR/pid")" || true
      for _ in 1 2 3 4 5; do is_up || break; sleep 1; done
      is_up && kill -9 "$(cat "$DIR/pid")"
    fi
    rm -rf "$DIR"
    echo down
    ;;
  *) sed -n 2,11p "$0"; exit 2 ;;
esac
