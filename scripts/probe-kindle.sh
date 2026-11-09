#!/bin/sh
# G0 — read-only Kindle device profile (spec §2, §16).
#
# Run ON THE KINDLE from your existing SSH session:
#     sh probe-kindle.sh > kt5-probe.txt
# or from the laptop without copying anything:
#     ssh kindle 'sh -s' < scripts/probe-kindle.sh > kt5-probe.txt
#
# Only inspects state; changes nothing. Output is redacted for MAC addresses,
# ip=/addr= values and serial-like fields, but READ IT before sharing.
# (Version numbers such as 5.16.21.0.1 are deliberately kept.)
set -u

redact() {
    sed -E \
        -e 's/([0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}/<MAC>/g' \
        -e 's/(ip|addr|inet|nfsroot)([=: ]+)[0-9][0-9.:]*/\1\2<IP>/g' \
        -e 's/(Serial|serial|SERIAL|serialno|androidboot.serialno|DSN|dsn)[[:space:]]*[:=][[:space:]]*[^[:space:]]+/\1=<REDACTED>/g'
}

section() { printf '\n=== %s ===\n' "$1"; }

{
section 'CPU / kernel'
uname -a
uname -m
cat /proc/cpuinfo 2>/dev/null | head -n 70

section 'RAM'
grep -E 'MemTotal|MemFree|MemAvailable|Cached|SwapTotal|SwapFree' /proc/meminfo

section 'DISPLAY'
cat /proc/fb 2>/dev/null || true
ls -l /dev/fb* 2>/dev/null || true
for f in /sys/class/graphics/fb0/virtual_size /sys/class/graphics/fb0/bits_per_pixel /sys/class/graphics/fb0/stride /sys/class/graphics/fb0/rotate; do
    [ -r "$f" ] && printf '%s: %s\n' "$f" "$(cat "$f")"
done

section 'INPUT'
ls -l /dev/input/event* 2>/dev/null || true
cat /proc/bus/input/devices 2>/dev/null | head -n 100

section 'FIRMWARE'
cat /etc/version.txt 2>/dev/null || true
cat /etc/prettyversion.txt 2>/dev/null || true

section 'KERNEL CMDLINE (redacted)'
cat /proc/cmdline 2>/dev/null || true

section 'PARTITIONS'
head -n 15 /proc/partitions 2>/dev/null || true

section 'USER STORAGE MOUNT (watch for noexec)'
mount | grep -E ' /mnt/us | /mnt/base-us ' || true

section 'FREE STORAGE'
df -h /mnt/us 2>/dev/null || true

section 'EXECUTABLE ABI HINTS'
for b in /bin/busybox /usr/bin/lipc-get-prop /bin/sh; do
    [ -e "$b" ] && { printf '%s: ' "$b"; head -c 20 "$b" | od -An -tx1 | head -n 2 | tr -s ' '; }
done
ls -l /lib/ld-* /lib/ld-linux* 2>/dev/null || true

section 'READER PROCESSES (observation only)'
ps 2>/dev/null | grep -E 'cvm|awesome|KPP|pillow|reader.lua|koreader|lipc|framework|xochitl' | grep -v grep || true

section 'DONE'
echo 'Copy relevant values into docs/kt5-hardware-report.md (no serials, MACs, IPs).'
} 2>&1 | redact
