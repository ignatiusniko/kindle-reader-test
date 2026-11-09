#!/bin/sh
# MEM-01 against the musl allocator setup used for the Kindle, run natively
# on an x86_64 host (no emulator noise in RSS numbers).
#     scripts/musl-memtest.sh [TURNS]
set -eu
cd "$(dirname "$0")/.."
TURNS="${1:-2000}"
LIST="$(mktemp)"
trap 'rm -f "$LIST"' EXIT
cargo zigbuild --tests --target x86_64-unknown-linux-musl -p reader-core --message-format=json \
    | python3 -c '
import sys, json
for line in sys.stdin:
    try:
        m = json.loads(line)
    except ValueError:
        continue
    if m.get("reason") == "compiler-artifact" and m.get("executable") \
            and m["profile"]["test"] and "/mem01-" in m["executable"]:
        print(m["executable"])
' > "$LIST"
EINK_MEM_TURNS="$TURNS" "$(head -n 1 "$LIST")" --nocapture
