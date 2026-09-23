#!/usr/bin/env bash
# Build the .deb into a staging DESTDIR (no root required).
# Usage: packaging/make-deb.sh <binary-path> <version> <outdir>
set -euo pipefail
BIN="${1:?binary path}"; VERSION="${2:?version}"; OUT="${3:?outdir}"
ROOT="$(mktemp -d)"
trap 'rm -rf "$ROOT"' EXIT
LIB="$ROOT/usr/lib/runwaybar"
mkdir -p "$ROOT/usr/bin" "$ROOT/usr/share/applications" "$ROOT/usr/share/icons" "$LIB"
install -m 0755 "$BIN" "$ROOT/usr/bin/runwaybar"
test -x "$ROOT/usr/bin/runwaybar"
install -m 0644 packaging/in.applair.RunwayBar.desktop "$ROOT/usr/share/applications/"
cp -r packaging/icons/. "$ROOT/usr/share/icons/"
sed "s/VERSION_PLACEHOLDER/$VERSION/" packaging/deb/control > "$ROOT/DEBIAN-ctl"
mkdir -p "$ROOT/DEBIAN"
mv "$ROOT/DEBIAN-ctl" "$ROOT/DEBIAN/control"
mkdir -p "$OUT"
dpkg-deb --build --root-owner-group "$ROOT" "$OUT/runwaybar_${VERSION}_amd64.deb"
echo "built $OUT/runwaybar_${VERSION}_amd64.deb"
