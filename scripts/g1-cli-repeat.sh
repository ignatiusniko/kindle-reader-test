#!/bin/sh
# G1 — run the CLI probe 10 times ON THE KINDLE (spec §19).
# Usage (in /mnt/us/eink-reader-dev on the device): sh g1-cli-repeat.sh
set -u
BIN="${1:-./eink-reader-cli-probe}"
[ -x "$BIN" ] || chmod 755 "$BIN" 2>/dev/null
"$BIN" || { echo "G1_FAIL first run exit=$?"; exit 1; }
i=1
while [ "$i" -le 10 ]; do
    "$BIN" >/dev/null || { echo "G1_FAIL run $i exit=$?"; exit 1; }
    i=$((i+1))
done
echo 'G1_CLI_REPEAT_OK'
