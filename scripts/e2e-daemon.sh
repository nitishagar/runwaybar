#!/usr/bin/env bash
# Stage-2 e2e: daemon lifecycle against the loopback mock (no real credentials).
# Fails loudly on every violated expectation.
set -uo pipefail
cd "$(dirname "$0")/.."

FAIL=0
check() { # check <description> <condition-exit-code>
  if [ "$2" -eq 0 ]; then echo "PASS: $1"; else echo "FAIL: $1"; FAIL=1; fi
}

pkill -f dev-mock-server.py 2>/dev/null; sleep 0.3
PORT=$(python3 scripts/dev-mock-server.py 0 > /tmp/rwb-mock-port.txt 2>/dev/null & sleep 0.7; cat /tmp/rwb-mock-port.txt)
[ -n "$PORT" ]; check "mock server up on $PORT" $?

E=/tmp/rwb-e2e; rm -rf "$E"; mkdir -p "$E/cache" "$E/config" "$E/runtime"
export RUNWAYBAR_FAKE_HOME="$PWD/tests/fixtures/home"
export XDG_CACHE_HOME="$E/cache" XDG_CONFIG_HOME="$E/config" XDG_RUNTIME_DIR="$E/runtime"
export RUNWAYBAR_TEST_ALLOW_HTTP=1
export CLAUDE_USAGE_ENDPOINT="http://127.0.0.1:$PORT/claude"
export CODEX_USAGE_ENDPOINT="http://127.0.0.1:$PORT/codex"
export Z_AI_QUOTA_ENDPOINT="http://127.0.0.1:$PORT/zai"
export OPENCODE_USAGE_ENDPOINT="http://127.0.0.1:$PORT/opencode"
export MUSE_SUBSCRIPTION_ENDPOINT="http://127.0.0.1:$PORT/muse"

BIN=./target/release/runwaybar
"$BIN" serve --no-tray --interval 60 >/tmp/rwb-serve.log 2>&1 &
DPID=$!
sleep 2.5
kill -0 "$DPID" 2>/dev/null; check "daemon started and stays up" $?

OUT=$("$BIN" status --format text)
echo "$OUT" | grep -q "Claude Code: ok"; check "claude-code polled ok" $?
echo "$OUT" | grep -q "z.ai / ZCode: ok"; check "zai polled ok" $?
echo "$OUT" | grep -q "OpenCode: ok"; check "opencode polled ok" $?
echo "$OUT" | grep -q "Muse Code: ok"; check "muse polled ok" $?
echo "$OUT" | grep -q "session  62%"; check "session percent present" $?

W1=$( { /usr/bin/time -f "%e" "$BIN" status --format json >/dev/null; } 2>&1 )
awk -v t="$W1" 'BEGIN { exit !(t+0 < 0.1) }'; check "IPC status under 100ms (was ${W1}s)" $?

RSS=$(ps -o rss= -p "$DPID" | tr -d ' ')
[ -n "$RSS" ] && [ "$RSS" -le 25600 ]; check "daemon RSS <= 25MB (${RSS}kB)" $?

timeout 5 "$BIN" serve --no-tray >/tmp/rwb-second.log 2>&1
[ $? -eq 0 ]; check "second serve exits 0" $?
grep -q "already running" /tmp/rwb-second.log; check "second serve prints notice" $?

"$BIN" refresh | grep -q "refresh requested"; check "refresh via IPC acked" $?

kill -TERM "$DPID" 2>/dev/null; sleep 1
! kill -0 "$DPID" 2>/dev/null; check "SIGTERM stops the daemon" $?
# recovery: files may remain, but a fresh serve must start (kernel-released flock + stale-socket unlink)
timeout 5 "$BIN" serve --no-tray --interval 60 >/tmp/rwb-restart.log 2>&1 &
RPID=$!; sleep 2
kill -0 "$RPID" 2>/dev/null; check "restart after crash-files works" $?
kill -TERM "$RPID" 2>/dev/null

pkill -f dev-mock-server.py 2>/dev/null
if [ "$FAIL" -eq 0 ]; then echo "e2e: ALL PASS"; else echo "e2e: FAILURES PRESENT"; fi
exit "$FAIL"
