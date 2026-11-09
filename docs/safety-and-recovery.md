# Safety and recovery

## Non-negotiables

- Existing jailbreak only; no firmware changes, no new jailbreak.
- Never write to system partitions, `/etc`, init scripts or KOReader files.
- Never `killall` Amazon services to test a CLI binary.
- No boot-time autostart for experimental builds.
- Graphics only with an **open, independent SSH session** and a rehearsed
  restore procedure (G3).

## Gate order

| Gate | What | Tooling here |
|---|---|---|
| G0 | Read-only device profile | `scripts/probe-kindle.sh`, `docs/kt5-hardware-report.md` |
| G1 | CLI binary runs 10× | `eink-reader-cli-probe`, `scripts/g1-cli-repeat.sh` |
| G2 | Slint binary built and transferred, **not run** | `scripts/build-kindle.sh`, `scripts/deploy-kindle.sh` |
| G3 | Manual, reversible display hand-off + restore | `scripts/run-graphics.sh.template` (placeholders refuse to run), `scripts/sample-recovery-checklist.md` |
| G4 | Hardware test window: draw, tap ×20, corners, grey steps, exit ×10, sleep/resume | `eink-reader-m0` via the verified launcher |
| G5 | Library/Reader UX | `eink-reader` |
| G6 | CBZ workload, MEM-01/02, ERR-01 | fixtures + `scripts/sample-memory.sh` |
| G7 | Worker kill, failed launch, idle battery, update/rollback | `render-worker`, `deploy-kindle.sh` rollback |

## G3: building the launcher

1. With the Amazon UI active, observe (read-only) which processes exist and
   which one owns the framebuffer/touch (spec §21.1). Record in the hardware
   report. Do not assume names from other Kindle models.
2. Write `handoff_display` and `restore_display` in a copy of
   `run-graphics.sh.template`. `restore_display` must be idempotent and safe
   after a partial hand-off.
3. Rehearse hand-off + restore **without** the app, from SSH, several times.
4. Only then set `VERIFIED=1` and try `APP=…/eink-reader-m0`.

The template already provides: single-instance lock, PID file, cleanup that
runs exactly once on EXIT/INT/TERM/HUP, SIGTERM → wait → SIGKILL of **only**
the app PID. A shell trap cannot help if the launcher itself is SIGKILLed or
power is lost — that is what the independent recovery path is for.

The app itself turns SIGTERM/SIGINT/SIGHUP into a normal exit (progress is
flushed). The backend grabs the touchscreen while running and releases it
when the process exits.

## Stop-and-recover decision tree

```text
Problem occurs
  ├── SSH still works?
  │     ├── YES → tap Exit, or `kill -TERM $(pidof eink-reader)`
  │     │         → if still running: `kill -KILL <that pid>` only
  │     │         → run the *verified* restore routine
  │     │         → confirm Amazon UI and KOReader
  │     └── NO  → do not try blind remote commands;
  │               follow your pre-tested physical recovery route
  └── Once recovered → collect redacted logs → classify → fix on the host
```

## Rollback

`deploy-kindle.sh` keeps `<binary>.previous` for every binary and prints the
rollback command. State (`data/state.json`) is versioned and never deleted;
an unreadable file is moved aside as `state.corrupt-<time>.json`.

## Uninstall

Remove `/mnt/us/eink-reader-dev` (app-owned files only). Books are never
written by the app.
