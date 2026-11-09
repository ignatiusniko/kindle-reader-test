#!/bin/sh
# G1/G2 — cross-build every binary for the Kindle and stage a dev bundle.
#
#     scripts/build-kindle.sh [TARGET]
#
# TARGET defaults to armv7-unknown-linux-musleabihf, the backend's documented
# target. Confirm it against the G0 probe (uname -m, ELF ABI) first; a
# 64-bit capable SoC may still need a 32-bit userspace binary.
# Requires: rustup target, zig, cargo-zigbuild, assets/fonts/ReaderFont.ttf.
set -eu
TARGET="${1:-armv7-unknown-linux-musleabihf}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [ ! -f assets/fonts/ReaderFont.ttf ]; then
    echo "missing assets/fonts/ReaderFont.ttf — run scripts/fetch-font.sh first" >&2
    exit 1
fi
command -v cargo-zigbuild >/dev/null || { echo "cargo-zigbuild not installed" >&2; exit 1; }

cargo zigbuild --release --target "$TARGET" \
    -p eink-reader-cli-probe -p render-worker
cargo zigbuild --release --target "$TARGET" \
    -p eink-reader --features eink-reader/kindle \
    -p eink-reader-m0 --features eink-reader-m0/kindle

OUT="target/$TARGET/release"
DIST="dist/eink-reader-dev"
rm -rf "$DIST"
mkdir -p "$DIST/logs"
for b in eink-reader-cli-probe eink-reader-m0 eink-reader render-worker; do
    cp "$OUT/$b" "$DIST/"
done
cp scripts/g1-cli-repeat.sh scripts/sample-memory.sh "$DIST/"
cp scripts/run-graphics.sh.template "$DIST/"
cp assets/fonts/LICENSE* "$DIST/" 2>/dev/null || true
(
    cd "$DIST"
    if command -v sha256sum >/dev/null; then
        sha256sum eink-reader-cli-probe eink-reader-m0 eink-reader render-worker > SHA256SUMS
    else
        shasum -a 256 eink-reader-cli-probe eink-reader-m0 eink-reader render-worker > SHA256SUMS
    fi
)
git rev-parse --short HEAD > "$DIST/REVISION" 2>/dev/null || echo unknown > "$DIST/REVISION"
echo "$TARGET" > "$DIST/TARGET"

echo
file "$DIST"/eink-reader-cli-probe "$DIST"/eink-reader-m0 "$DIST"/eink-reader "$DIST"/render-worker 2>/dev/null || true
ls -lh "$DIST"
echo
echo "Staged in $DIST. Nothing has been sent to or run on the device."
