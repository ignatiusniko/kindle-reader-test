#!/bin/sh
# KUAL entry point (spec §25.2). Starts the verified launcher detached from
# KUAL, so KUAL can close before the Kindle UI is paused. Never starts the
# app directly and never runs at boot.
#
#     sh launch.sh reader | reader-log | hwtest
set -u
DIR="${EINK_DIR:-/mnt/us/eink-reader-dev}"
LAUNCHER="$DIR/run-graphics.sh"
KLOG="$DIR/logs/kual.log"
# Seconds to let KUAL's menu close before the screen is handed over.
DELAY="${EINK_KUAL_DELAY:-2}"

mkdir -p "$DIR/logs"
log() { echo "$(date) $*" >>"$KLOG"; }

case "${1:-reader}" in
    reader) APP="$DIR/eink-reader"; APP_ARGS="" ;;
    reader-log) APP="$DIR/eink-reader"; APP_ARGS="--log" ;;
    hwtest) APP="$DIR/eink-reader-m0"; APP_ARGS="" ;;
    *) log "unknown mode: $1"; exit 2 ;;
esac

if [ ! -f "$LAUNCHER" ]; then
    log "missing $LAUNCHER — run scripts/deploy-kindle.sh"
    exit 1
fi
if [ ! -x "$APP" ]; then
    log "missing or not executable: $APP"
    exit 1
fi
if pidof eink-reader eink-reader-m0 >/dev/null 2>&1; then
    log "already running; not starting a second instance"
    exit 0
fi

log "launch ${1:-reader}"
export APP APP_ARGS
run() {
    sleep "$DELAY"
    exec /bin/sh "$LAUNCHER"
}
# Detach: own session where available, no terminal, output to the log.
if command -v setsid >/dev/null 2>&1; then
    setsid sh -c "sleep $DELAY; exec /bin/sh '$LAUNCHER'" </dev/null >>"$KLOG" 2>&1 &
else
    run </dev/null >>"$KLOG" 2>&1 &
fi
exit 0
