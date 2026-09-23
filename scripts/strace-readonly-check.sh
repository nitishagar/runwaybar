#!/usr/bin/env bash
# Read-only-credentials guard (invariant #1): runs the integration suite under strace
# with a fixture HOME and fails if any write-path syscall touches a credential file.
# Usage: scripts/strace-readonly-check.sh   (from the repo root; needs strace + cargo)
set -euo pipefail

cd "$(dirname "$0")/.."

FIXTURE_HOME="$PWD/tests/fixtures/home"
LOG="$(mktemp /tmp/runwaybar-strace.XXXXXX.log)"
trap 'rm -f "$LOG"' EXIT

# Snapshot mtimes before, so we can also prove the files were not modified.
BEFORE="$(find "$FIXTURE_HOME" -type f -exec stat -c '%n %Y %s' {} \; | sort)"

# Run the fixture-driven integration test under strace, following children,
# tracing file-opening writes and renames into the fixture home.
strace -f -q -e trace=openat,rename,renameat,renameat2,unlink,unlinkat,creat,open \
  -o "$LOG" \
  env RUNWAYBAR_FAKE_HOME="$FIXTURE_HOME" \
      XDG_CACHE_HOME="$(mktemp -d)" \
      XDG_CONFIG_HOME="$(mktemp -d)" \
      RUNWAYBAR_TEST_ALLOW_HTTP=1 \
  cargo test --test oneshot_burst --test absent_tools -- --test-threads=1 >/dev/null 2>&1

AFTER="$(find "$FIXTURE_HOME" -type f -exec stat -c '%n %Y %s' {} \; | sort)"

fail=0
# Any write-mode open or destructive syscall against the fixture credential files.
if grep -E "$FIXTURE_HOME" "$LOG" | grep -E 'O_WRONLY|O_RDWR|O_TRUNC|O_CREAT|rename|unlink|creat' >/dev/null; then
  echo "FAIL: write-path syscalls against credential files:"
  grep -E "$FIXTURE_HOME" "$LOG" | grep -E 'O_WRONLY|O_RDWR|O_TRUNC|O_CREAT|rename|unlink|creat' | head -20
  fail=1
fi

if [ "$BEFORE" != "$AFTER" ]; then
  echo "FAIL: fixture credential files changed:"
  diff <(echo "$BEFORE") <(echo "$AFTER") || true
  fail=1
fi

if [ "$fail" -eq 0 ]; then
  echo "OK: no write-path syscalls against credential files; files unchanged"
fi
exit "$fail"
