#!/usr/bin/env bash
# Stage-2 e2e: daemon lifecycle against the loopback mock (no real credentials).
set -uo pipefail
cd "$(dirname "$0")/.."

pkill -f dev-mock-server.py 2>/dev/null
sleep 0.3
PORT=$(python3 scripts/dev-mock-server.py 0 > /tmp/rwb-mock-port.txt 2>/dev/null & sleep 0.7; cat /tmp/rwb-mock-port.txt)
echo "mock port=$PORT"

E=/tmp/rwb-e2e; rm -rf "$E"; mkdir -p "$E/cache" "$E/config"
export RUNWAYBAR_FAKE_HOME="$PWD/tests/fixtures/home"
export XDG_CACHE_HOME="$E/cache"
export XDG_CONFIG_HOME="$E/config"
export RUNWAYBAR_TEST_ALLOW_HTTP=1
export CLAUDE_USAGE_ENDPOINT="http://127.0.0.1:$PORT/claude"
export CODEX_USAGE_ENDPOINT="http://127.0.0.1:$PORT/codex"
export Z_AI_QUOTA_ENDPOINT="http://127.0.0.1:$PORT/zai"
export OPENCODE_USAGE_ENDPOINT="http://127.0.0.1:$PORT/opencode"
unset XDG_RUNTIME_DIR

BIN=./target/release/runwaybar
"$BIN" serve --no-tray --interval 60 &
DPID=$!
sleep 2.5

echo "--- status via IPC (timed x2):"
/usr/bin/time -f "  wall1=%es" "$BIN" status --format text 2>&1 | tail -8
/usr/bin/time -f "  wall2=%es" "$BIN" status --format waybar 2>&1 | tail -2

echo "--- daemon idle RSS:"
ps -o rss= -p "$DPID" | awk '{print "  rss_kb=" $1}'

echo "--- second serve attempt:"
timeout 5 "$BIN" serve --no-tray; echo "  second-exit=$?"

echo "--- refresh via IPC:"
"$BIN" refresh

echo "--- SIGTERM cleanup:"
kill -TERM "$DPID" 2>/dev/null
sleep 1
TMPD=/tmp/runwaybar-$(id -u)
echo "  runtime dir after TERM: $(ls "$TMPD" 2>/dev/null | tr '\n' ' ')"
echo "  daemon alive? $(kill -0 "$DPID" 2>/dev/null && echo yes || echo no)"
pkill -f dev-mock-server.py 2>/dev/null
true
