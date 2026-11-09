#!/bin/sh
# Copy the staged bundle to the Kindle's user storage (spec §20.5, §25.1, §25.3).
#
#     KINDLE=kindle scripts/deploy-kindle.sh
#
# * Deployment only — this script NEVER launches anything.
# * Writes only to $REMOTE_DIR on user storage (default /mnt/us/eink-reader-dev)
#   and the KUAL extension folder /mnt/us/extensions/eink-reader.
# * Uploads as *.new, verifies SHA-256 on the device, keeps the previous
#   binaries as *.previous, then swaps with mv. Refuses while the app runs.
set -eu
KINDLE="${KINDLE:-kindle}"
REMOTE_DIR="${REMOTE_DIR:-/mnt/us/eink-reader-dev}"
EXT_DIR=/mnt/us/extensions/eink-reader
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIST="$ROOT/dist/eink-reader-dev"
EXT="$ROOT/dist/extensions/eink-reader"
BINS="eink-reader-cli-probe eink-reader-m0 eink-reader render-worker"

[ -f "$DIST/SHA256SUMS" ] || { echo "run scripts/build-kindle.sh first" >&2; exit 1; }

case "$REMOTE_DIR" in
    /mnt/us/*) ;;
    *) echo "REMOTE_DIR must be under /mnt/us (user storage)" >&2; exit 1 ;;
esac

ssh "$KINDLE" "uname -m; echo SSH_OK" | grep -q SSH_OK || { echo "SSH check failed; stop here (spec §17)" >&2; exit 1; }

if ssh "$KINDLE" "pidof eink-reader eink-reader-m0 render-worker >/dev/null 2>&1"; then
    echo "the app is running on the device; exit it first (never overwrite a running binary)" >&2
    exit 1
fi

ssh "$KINDLE" "mkdir -p '$REMOTE_DIR/logs'"
for b in $BINS; do
    scp -O "$DIST/$b" "$KINDLE:$REMOTE_DIR/$b.new"
done
scp -O "$DIST/SHA256SUMS" "$DIST/REVISION" "$DIST/TARGET" \
    "$DIST/g1-cli-repeat.sh" "$DIST/sample-memory.sh" "$DIST/run-graphics.sh.template" \
    "$KINDLE:$REMOTE_DIR/"
scp -O "$DIST/run-graphics.sh" "$KINDLE:$REMOTE_DIR/run-graphics.sh.new"

# KUAL menu entry (only plain files under the extension folder).
ssh "$KINDLE" "mkdir -p '$EXT_DIR/bin'"
scp -O "$EXT/config.xml" "$EXT/menu.json" "$KINDLE:$EXT_DIR/"
scp -O "$EXT/bin/launch.sh" "$KINDLE:$EXT_DIR/bin/"

# Verify on device, then activate. sha256sum is part of busybox on most
# jailbroken Kindles; if it is missing, compare sizes and stop.
ssh "$KINDLE" "cd '$REMOTE_DIR' && set -e
    if command -v sha256sum >/dev/null 2>&1; then
        sed 's/  \(.*\)\$/  \1.new/' SHA256SUMS | sha256sum -c -
    else
        echo 'sha256sum missing on device: verify manually before activating' >&2; exit 1
    fi
    for b in $BINS run-graphics.sh; do
        [ -f \"\$b\" ] && mv -f \"\$b\" \"\$b.previous\"
        mv -f \"\$b.new\" \"\$b\"
        chmod 755 \"\$b\"
    done
    chmod 755 '$EXT_DIR/bin/launch.sh'
    ls -l"
echo "Deployed to $KINDLE:$REMOTE_DIR and $EXT_DIR (not launched)."
echo "On the Kindle: KUAL -> E-Ink Reader (Experimental) -> Open reader."
echo "Roll back with:"
echo "  ssh $KINDLE 'cd $REMOTE_DIR && for b in $BINS; do mv -f \$b.previous \$b; done'"
