#!/usr/bin/env bash
# Build a release tarball with README + LICENSE + sha256.
# Usage: packaging/make-tarball.sh <binary-path> <version> <target-triple> <outdir>
set -euo pipefail
BIN="${1:?binary path}"; VERSION="${2:?version}"; TRIPLE="${3:?target triple}"; OUT="${4:?outdir}"
NAME="runwaybar-$VERSION-$TRIPLE"
STAGE="$(mktemp -d)/$NAME"
trap 'rm -rf "$(dirname "$STAGE")"' EXIT
mkdir -p "$STAGE"
install -m 0755 "$BIN" "$STAGE/runwaybar"
install -m 0644 README.md LICENSE "$STAGE/"
mkdir -p "$OUT"
tar -C "$(dirname "$STAGE")" -czf "$OUT/$NAME.tar.gz" "$NAME"
(cd "$OUT" && sha256sum "$NAME.tar.gz" >> SHA256SUMS.txt.part)
echo "built $OUT/$NAME.tar.gz"
