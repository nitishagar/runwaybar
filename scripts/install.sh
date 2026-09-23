#!/usr/bin/env bash
# RunwayBar installer: downloads the latest release tarball, verifies its sha256,
# installs to ~/.local/bin. Override the source with --base <dir-or-url> for testing.
set -euo pipefail
# Note: the checksum detects corruption and casual tampering only; release artifacts
# are additionally signed with GitHub attestations (release page → attestations).
BASE=""
while [ $# -gt 0 ]; do
  case "$1" in
    --base) BASE="$2"; shift 2 ;;
    *) echo "usage: install.sh [--base <url-or-dir>]" >&2; exit 64 ;;
  esac
done

ARCH="$(uname -m)"
case "$ARCH" in
  x86_64) NAME="x86_64-linux" ;;
  aarch64|arm64) NAME="aarch64-musl-static" ;;
  *) echo "unsupported architecture: $ARCH" >&2; exit 1 ;;
esac

DEST="${HOME}/.local/bin"
mkdir -p "$DEST"

if [ -n "$BASE" ]; then
  # local testing mode: BASE is a directory with the artifacts
  if [ ! -d "$BASE" ]; then echo "--base directory not found: $BASE" >&2; exit 1; fi
  fetch() { cp "$1" "$2"; }
else
  BASE="https://github.com/nitishagar/runwaybar/releases/latest/download"
  fetch() { curl -fL --retry 3 -o "$2" "$1"; }
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
fetch "$BASE/SHA256SUMS.txt" "$TMP/SHA256SUMS.txt"
LINE="$(grep -E "runwaybar-[0-9.]+-$NAME.tar.gz$" "$TMP/SHA256SUMS.txt" | head -1 || true)"
if [ -z "$LINE" ]; then echo "no tarball for $NAME in SHA256SUMS.txt" >&2; exit 1; fi
TARBALL="$(awk '{print $2}' <<<"$LINE")"
fetch "$BASE/$TARBALL" "$TMP/$TARBALL"
(cd "$TMP" && sha256sum -c <(echo "$LINE"))
tar -xzf "$TMP/$TARBALL" -C "$TMP"
BIN="$(find "$TMP" -maxdepth 2 -name runwaybar -type f | head -1)"
install -m 0755 "$BIN" "$DEST/runwaybar"
echo "installed $DEST/runwaybar"
case ":$PATH:" in
  *":$DEST:"*) ;;
  *) echo "note: add $DEST to PATH" ;;
esac
