#!/bin/sh
# Host test for scripts/run-graphics-kt5.sh and the KUAL entry point.
# Uses a fake app and a fake "Kindle UI" process (a renamed `sleep`) to
# check hand-off (process paused), restore (resumed), locking, signals and
# argument passing. Touches no real Kindle processes. Linux only (/proc).
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
T="$(mktemp -d)"
FAIL=0
UIPID=""
trap 'kill -CONT $UIPID 2>/dev/null; kill $UIPID 2>/dev/null; rm -rf "$T"' EXIT

ok() { echo "ok   - $1"; }
bad() { echo "FAIL - $1"; FAIL=1; }
check() { if eval "$2"; then ok "$1"; else bad "$1"; fi; }
state() { awk '{print $3}' "/proc/$1/stat" 2>/dev/null; }

export EINK_DIR="$T/dev"
mkdir -p "$EINK_DIR"
cp "$ROOT/scripts/run-graphics-kt5.sh" "$EINK_DIR/run-graphics.sh"

# Fake UI process with a unique name.
UINAME="einkfakeui$$"
cp "$(command -v sleep)" "$T/$UINAME"
"$T/$UINAME" 300 &
UIPID=$!
export UI_PROCS="$UINAME"

# Fake app: records its args and the UI state while it runs.
cat >"$EINK_DIR/eink-reader" <<APP
#!/bin/sh
echo "args:\$*" >"$T/app.out"
awk '{print \$3}' /proc/$UIPID/stat >"$T/ui-during"
trap 'echo term >"$T/app.term"; exit 0' TERM
i=0; while [ "\$i" -lt "\${FAKE_APP_SECONDS:-1}0" ]; do sleep 0.1; i=\$((i+1)); done
exit "\${FAKE_APP_STATUS:-0}"
APP
chmod +x "$EINK_DIR/eink-reader"
LAUNCH="$EINK_DIR/run-graphics.sh"

# 1. Rehearsal: UI paused, then resumed; lock cleaned up.
( MODE=rehearse sh "$LAUNCH" >"$T/rehearse.out" 2>&1 ) &
RP=$!
sleep 1
check "rehearse pauses UI" '[ "$(state $UIPID)" = T ]'
wait $RP
check "rehearse resumes UI" '[ "$(state $UIPID)" != T ]'
check "rehearse removes lock" '[ ! -e "$EINK_DIR/logs/launcher.lock" ]'

# 2. Normal run: args passed, UI paused during run, exit status propagated.
FAKE_APP_STATUS=3 APP_ARGS="--log" sh "$LAUNCH" >/dev/null 2>&1
st=$?
check "exit status propagated" '[ "$st" = 3 ]'
check "APP_ARGS passed to app" 'grep -q "args:--log" "$T/app.out"'
check "UI paused while app runs" '[ "$(cat $T/ui-during)" = T ]'
check "UI resumed after app exit" '[ "$(state $UIPID)" != T ]'
check "lock removed after run" '[ ! -e "$EINK_DIR/logs/launcher.lock" ]'

# 3. Live lock refuses a second launcher; stale lock is recovered.
mkdir -p "$EINK_DIR/logs/launcher.lock"; echo $$ >"$EINK_DIR/logs/launcher.lock/pid"
sh "$LAUNCH" >/dev/null 2>&1
check "live lock refuses second instance" '[ $? -ne 0 ] && [ -e "$EINK_DIR/logs/launcher.lock" ]'
echo 999999 >"$EINK_DIR/logs/launcher.lock/pid"
FAKE_APP_SECONDS=0 sh "$LAUNCH" >/dev/null 2>&1
check "stale lock recovered" '[ $? -eq 0 ] && [ ! -e "$EINK_DIR/logs/launcher.lock" ]'

# 4. SIGTERM to the launcher: app asked to exit, UI restored.
rm -f "$T/app.term" "$T/app.out"
( FAKE_APP_SECONDS=20 sh "$LAUNCH" >/dev/null 2>&1 ) &
# Wait until the app is really running (hand-off can take ~1 s).
i=0; while [ ! -f "$T/app.out" ] && [ $i -lt 50 ]; do sleep 0.1; i=$((i+1)); done
LP="$(cat "$EINK_DIR/logs/launcher.lock/pid" 2>/dev/null)"
kill -TERM "$LP" 2>/dev/null
sleep 1.5
check "TERM reaches the app" '[ -f "$T/app.term" ]'
check "UI resumed after TERM" '[ "$(state $UIPID)" != T ]'
check "lock removed after TERM" '[ ! -e "$EINK_DIR/logs/launcher.lock" ]'

# 5. KUAL entry point: returns at once, app runs detached, then cleans up.
rm -f "$T/app.out"
start=$(date +%s)
EINK_KUAL_DELAY=0 FAKE_APP_SECONDS=1 sh "$ROOT/kual/eink-reader/bin/launch.sh" reader-log
check "KUAL launch returns immediately" '[ $(( $(date +%s) - start )) -le 1 ]'
i=0; while [ ! -f "$T/app.out" ] && [ $i -lt 30 ]; do sleep 0.1; i=$((i+1)); done
check "KUAL starts app with --log" 'grep -q "args:--log" "$T/app.out"'
i=0; while [ -e "$EINK_DIR/logs/launcher.lock" ] && [ $i -lt 50 ]; do sleep 0.1; i=$((i+1)); done
check "KUAL run restores UI" '[ "$(state $UIPID)" != T ]'
check "KUAL log written" 'grep -q "launch reader-log" "$EINK_DIR/logs/kual.log"'
sh "$ROOT/kual/eink-reader/bin/launch.sh" bogus
check "KUAL rejects unknown mode" '[ $? -eq 2 ]'

[ "$FAIL" = 0 ] && echo "all launcher tests passed" || { echo "launcher tests FAILED"; exit 1; }
