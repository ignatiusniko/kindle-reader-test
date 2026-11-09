#!/bin/sh
# Read-only memory sampler for MEM-01 and friends (spec §26.5).
# Usage ON THE KINDLE, from a separate SSH session:
#     sh sample-memory.sh <PID> [SECONDS] [WORKER_PID]
# Prints one line per second. Do not run for hours (flash wear if redirected).
PID="$1"
COUNT="${2:-60}"
WPID="${3:-}"
i=0
while [ "$i" -lt "$COUNT" ] && [ -r "/proc/$PID/status" ]; do
    printf '%s ' "$(date +%s)"
    awk '/^VmRSS:/ {printf "ui_rss_kib=%s ", $2}
         /^VmHWM:/ {printf "ui_hwm_kib=%s ", $2}' "/proc/$PID/status"
    if [ -n "$WPID" ] && [ -r "/proc/$WPID/status" ]; then
        awk '/^VmRSS:/ {printf "worker_rss_kib=%s ", $2}
             /^VmHWM:/ {printf "worker_hwm_kib=%s ", $2}' "/proc/$WPID/status"
    fi
    awk '/^MemAvailable:/ {printf "available_kib=%s", $2}' /proc/meminfo
    printf '\n'
    i=$((i+1))
    sleep 1
done
