#!/bin/sh
# Provide assets/fonts/ReaderFont.ttf (needed for `--features kindle`).
#
# Uses DejaVu Sans (Bitstream Vera licence + public-domain changes: freely
# redistributable). Copies from the system if installed, otherwise downloads
# the upstream release. Never use a proprietary Kindle system font.
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/assets/fonts"
mkdir -p "$DEST"
for f in /usr/share/fonts/truetype/dejavu/DejaVuSans.ttf /usr/share/fonts/TTF/DejaVuSans.ttf /usr/local/share/fonts/DejaVuSans.ttf; do
    if [ -f "$f" ]; then
        cp "$f" "$DEST/ReaderFont.ttf"
        echo "copied $f"
        LIC=/usr/share/doc/fonts-dejavu-core/copyright
        [ -f "$LIC" ] && cp "$LIC" "$DEST/LICENSE-DejaVu.txt"
        break
    fi
done
if [ ! -f "$DEST/ReaderFont.ttf" ]; then
    URL='https://github.com/dejavu-fonts/dejavu-fonts/releases/download/version_2_37/dejavu-fonts-ttf-2.37.tar.bz2'
    TMP="$(mktemp -d)"
    trap 'rm -rf "$TMP"' EXIT
    echo "downloading $URL"
    curl -fsSL "$URL" -o "$TMP/dejavu.tar.bz2"
    tar -xjf "$TMP/dejavu.tar.bz2" -C "$TMP"
    cp "$TMP"/dejavu-fonts-ttf-2.37/ttf/DejaVuSans.ttf "$DEST/ReaderFont.ttf"
    cp "$TMP"/dejavu-fonts-ttf-2.37/LICENSE "$DEST/LICENSE-DejaVu.txt"
fi
[ -f "$DEST/LICENSE-DejaVu.txt" ] || echo "NOTE: add the DejaVu licence text as assets/fonts/LICENSE-DejaVu.txt before distributing"
ls -l "$DEST"
