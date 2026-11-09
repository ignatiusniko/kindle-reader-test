#!/bin/sh
# Launcher for Kindle Basic 2022 (KT5) on firmware 5.18.5.0.1, built from
# run-graphics.sh.template. Hands the screen to the reader and restores
# the Kindle UI on every exit path (normal exit, crash, INT/TERM/HUP).
#
# Verified on one device (KT5, 5.18.5.0.1): hand-off and restore rehearsed,
# hardware test and reader launched. On any other model or firmware,
# re-verify the process names first (docs/safety-and-recovery.md).
#
# Usage (normally via deploy-kindle.sh as run-graphics.sh, or KUAL):
#     sh run-graphics.sh                         # reader
#     APP_ARGS=--log sh run-graphics.sh          # reader with timing log
#     APP=./eink-reader-m0 sh run-graphics.sh    # hardware test window
#     MODE=rehearse sh run-graphics.sh           # hand-off + restore only
set -u

VERIFIED=1
DIR="${EINK_DIR:-/mnt/us/eink-reader-dev}"
APP="${APP:-$DIR/eink-reader}"
APP_ARGS="${APP_ARGS:-}"
MODE="${MODE:-run}"
LOG="$DIR/logs/ui.log"
PIDFILE="$DIR/logs/app.pid"
LOCK="$DIR/logs/launcher.lock"
# Kindle UI processes paused while the reader owns the screen.
UI_PROCS="${UI_PROCS:-awesome cvm KPPMainApp}"
mkdir -p "$DIR/logs"

# Terminate KOReader before we claim the framebuffer. A best-effort signal
# is not enough: if KOReader remains alive, its input grab and framebuffer
# writes can interfere with our app. Never force-kill it automatically:
# SIGKILL could lose reading progress or interrupt KOReader's cleanup.
stop_koreader() {
    pids="$(pidof reader.lua 2>/dev/null || true)"
    [ -n "$pids" ] || return 0
    echo "$(date) KOReader detected (reader.lua pid(s): $pids); requesting shutdown" >>"$LOG"
    # shellcheck disable=SC2086
    kill -TERM $pids 2>/dev/null || true

    # 50 x 0.1s = 5 seconds, configurable for host tests.
    tries=0
    max_tries="${KOREADER_STOP_TRIES:-50}"
    while pidof reader.lua >/dev/null 2>&1; do
        if [ "$tries" -ge "$max_tries" ]; then
            echo "KOReader is still running. Refusing display hand-off; exit KOReader manually." >&2
            echo "$(date) KOReader failed to exit; launch aborted" >>"$LOG"
            return 1
        fi
        sleep 0.1
        tries=$((tries + 1))
    done
    # Allow KOReader's shell launcher time to finish its own device cleanup.
    sleep 1
    echo "$(date) KOReader stopped; continuing display hand-off" >>"$LOG"
    return 0
}

handoff_display() {
    stop_koreader || return 1
    lipc-set-prop com.lab126.pillow disableEnablePillow disable 2>/dev/null
    # shellcheck disable=SC2086
    killall -STOP $UI_PROCS 2>/dev/null
    usleep 300000 2>/dev/null || sleep 1
}

restore_display() {
    # Idempotent: CONT on a running process is harmless.
    # shellcheck disable=SC2086
    killall -CONT $UI_PROCS 2>/dev/null
    lipc-set-prop com.lab126.pillow disableEnablePillow enable 2>/dev/null
    return 0
}

[ "$VERIFIED" = 1 ] || { echo 'refusing to run: launcher not verified (G3)' >&2; exit 1; }

# Single instance. A lock left behind by a power loss or SIGKILL is
# detected through the PID stored inside it and removed.
take_lock() {
    if mkdir "$LOCK" 2>/dev/null; then
        echo $$ >"$LOCK/pid"
        return 0
    fi
    old="$(cat "$LOCK/pid" 2>/dev/null || true)"
    if [ -n "$old" ] && kill -0 "$old" 2>/dev/null; then
        echo "another launcher is running (pid $old)" >&2
        return 1
    fi
    echo "removing stale launcher lock" >&2
    rm -rf "$LOCK"
    mkdir "$LOCK" 2>/dev/null && echo $$ >"$LOCK/pid"
}
take_lock || exit 1

CLEANED=0
cleanup() {
    [ "$CLEANED" = 1 ] && return
    CLEANED=1
    trap '' EXIT HUP INT TERM
    if [ -f "$PIDFILE" ]; then
        pid="$(cat "$PIDFILE")"
        # The app flushes progress on SIGTERM; only then force it.
        kill -TERM "$pid" 2>/dev/null
        n=0
        while kill -0 "$pid" 2>/dev/null && [ "$n" -lt 30 ]; do sleep 0.1; n=$((n+1)); done
        kill -KILL "$pid" 2>/dev/null
        rm -f "$PIDFILE"
    fi
    restore_display || echo 'WARNING: restore_display failed' >&2
    rm -rf "$LOCK"
    echo 'Kindle UI restored.'
}
trap 'cleanup' EXIT
trap 'cleanup; exit 130' INT
trap 'cleanup; exit 143' TERM HUP

handoff_display || exit 1
if [ "$MODE" = rehearse ]; then
    echo 'Hand-off done: the Kindle UI should be frozen. Restoring in 5 s...'
    sleep 5
    exit 0
fi
echo "$(date) start: $APP $APP_ARGS" >>"$LOG"
# shellcheck disable=SC2086
"$APP" $APP_ARGS >>"$LOG" 2>&1 &
echo $! >"$PIDFILE"
wait $!
status=$?
rm -f "$PIDFILE"
echo "$(date) app exited with status $status" >>"$LOG"
exit "$status"
