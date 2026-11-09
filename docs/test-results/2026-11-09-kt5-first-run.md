# 2026-11-09 — first run on a Kindle Basic 2022 (manual, user-reported)

Device: Kindle Basic 11th gen (2022), firmware 5.18.5.0.1, jailbroken,
USBNetLite SSH over Wi-Fi. Build: `armv7-unknown-linux-musleabihf` from
`main` after PR #1.

| Test | Result |
|---|---|
| SSH reachable (port 22) | yes, after keeping the device awake (Wi-Fi drops in sleep) |
| Deploy over Wi-Fi | first attempt dropped mid-transfer (device slept); nothing activated |
| Launcher hand-off / restore rehearsal | done by user |
| Hardware test window / reader launched through the launcher | yes |
| Open and read CBZ | works; user reports it is more responsive than KOReader |

Not measured yet (`null` = not measured; never guess):

```yaml
test_run:
  date: "2026-11-09"
  device: "Kindle Basic 11th Gen (2022)"
  firmware: "5.18.5.0.1"
  build_target: "armv7-unknown-linux-musleabihf"
  test_id: "UI-01/UX-01 (informal)"
  mem_available_start_kib: null
  ui_rss_peak_kib: null
  worker_rss_peak_kib: null
  render_p50_ms: null
  render_p95_ms: null
  passed: null
  notes: "Informal first run; formal G4 checklist (20 taps, corners, grey steps, 10x exit, sleep/resume) still to record."
```

Next: MEM-01 with `sample-memory.sh`, timing log (`--log`), sleep/resume,
repeated open/exit, first launch from KUAL.
