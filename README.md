# E-Ink Reader (experimental) — Kindle Basic 2022

A minimal, offline CBZ manga reader for E-Ink Kindles, written in Rust with a
Slint software-rendered UI. Built from *E-Ink Reader — Technical Spec v0.3*.

> **Status: experimental.** First device run on 2026-11-09: on one Kindle
> Basic 2022 (KT5) with firmware 5.18.5.0.1, the launcher's hand-off and
> restore were rehearsed, and the hardware test window and the reader opened
> and read CBZ books. Long-session memory, sleep/resume and the remaining
> G4–G7 checks have **not** been run yet (docs/test-results/). On any other
> model or firmware, re-establish gate G3 before launching anything.

## What works today (host-verified)

| Area | State |
|---|---|
| CBZ indexing from ZIP central directory only, natural page order | ✅ tested (1,500-entry archive, no page decode) |
| Bounded single-entry reads: size, ratio (zip-bomb), encryption, method checks | ✅ tested |
| JPEG decode with DCT scaling (1/2–1/8) straight to Gray8 | ✅ tested |
| PNG streamed row-by-row into a box filter (peak ≈ one row + output) | ✅ tested (2000×8000 RGBA under 1 MiB planned peak) |
| Reservation-based memory budget: `min(64 MiB, 35% MemAvailable)` | ✅ tested |
| Capacity-1 "latest wins" render scheduler, generation ids, retry once at lower quality | ✅ tested |
| Separate `render-worker` process with `RLIMIT_AS`; UI survives worker kill / abort / allocation failure | ✅ tested |
| Library → Reader → Reading Options UI (Slint), tap zones, RTL/LTR, Fit Page/Width with segments | ✅ headless UI flow test + snapshots |
| Per-book progress and prefs, atomic JSON writes, corrupt-state recovery, relink on file change | ✅ tested |
| Clean exit on SIGTERM/SIGINT/SIGHUP (progress flushed, launcher restore can run) | implemented |
| ARMv7 musl cross-build (`cargo zigbuild`) of all binaries | ✅ built; CLI probe and worker run under `qemu-arm` |
| musl allocator fragmentation found in MEM-01 → mimalloc on musl builds | ✅ fixed and re-measured (see `docs/decisions/0003-allocator-on-musl.md`) |
| Launcher hand-off/restore, display, touch and CBZ reading on KT5 (fw 5.18.5.0.1) | ✅ first manual run on one device |
| KUAL menu entry (Open reader / timing log / hardware test) | ✅ host-tested (`scripts/test-launcher.sh`); first device run pending |
| Long-session memory, sleep/resume, 50× open/exit, battery on KT5 | ❌ **not tested yet — G4–G7** |

## Repository layout

```text
apps/cli-probe        G1 smoke test (no display)
apps/kindle-hwtest    G2/G4 Slint test window: tap counter + exit (eink-reader-m0)
apps/kindle-reader    The reader UI (eink-reader); features: kindle | desktop
apps/render-worker    Isolated decode process (M3)
kual/eink-reader      KUAL menu entry (calls the verified launcher)
crates/memory-budget  Checked size math, limits, reservations, /proc/meminfo
crates/cbz-reader     Metadata-only index + bounded entry reads
crates/image-pipeline JPEG/PNG → display-sized Gray8 under a budget
crates/persistence    Progress + per-book prefs, atomic writes
crates/reader-core    Catalog, navigation, state machine, scheduler, IPC, controller
crates/diagnostic-kit RSS/HWM sampling, timings, bounded log
crates/test-fixtures  Synthetic CBZ generator (no copyrighted content)
ui/                   Library.slint, Reader.slint, ReadingOptions.slint, theme
scripts/              probe / build / deploy / memory sampler / launcher TEMPLATE
docs/                 Architecture, device matrix, safety runbook, tests, decisions
```

## Host quick start

Host builds of the Slint UI need the fontconfig headers (Debian/Ubuntu:
`libfontconfig-dev`; macOS needs nothing extra).

```bash
scripts/check.sh                 # fmt + clippy + all tests (same as CI)
```

```bash
scripts/snapshot.sh              # renders UI screens to dist/snapshots/*.png
```

Desktop preview window (laptop only; not evidence of Kindle behaviour):

```bash
cargo run -q -p test-fixtures --bin make-fixtures -- books
```

```bash
cargo run -p eink-reader --features desktop -- --books books --data /tmp/eink-data --size 536x724 --dpi 150
```

## Building for the Kindle (G1/G2 — build and transfer only)

1. Run the read-only probe and fill in `docs/kt5-hardware-report.md`
   (`ssh kindle 'sh -s' < scripts/probe-kindle.sh`). Confirm the target ABI.
2. Install the toolchain: `rustup target add armv7-unknown-linux-musleabihf`,
   Zig, and `cargo install cargo-zigbuild`.
3. Provide a redistributable font: `scripts/fetch-font.sh`.
4. `scripts/build-kindle.sh` → `dist/eink-reader-dev/` (+ `SHA256SUMS`).
5. `KINDLE=kindle scripts/deploy-kindle.sh` copies to
   `/mnt/us/eink-reader-dev` with checksum verification and a `.previous`
   rollback copy. It never launches anything.
6. On the device, run `sh g1-cli-repeat.sh` (G1). Expect `G1_CLI_REPEAT_OK`.
7. Graphics need gate G3. For **KT5 on firmware 5.18.5.0.1** the deploy
   step installs the device-verified launcher `run-graphics.sh`
   (`scripts/run-graphics-kt5.sh`). Rehearse it once on your device:
   `ssh kindle 'cd /mnt/us/eink-reader-dev && MODE=rehearse sh run-graphics.sh'`.
   For other models/firmware, start from `run-graphics.sh.template` and
   `scripts/sample-recovery-checklist.md` (`docs/safety-and-recovery.md`).
8. Start it from the Kindle: **KUAL → E-Ink Reader (Experimental) → Open
   reader**. Leave with **Exit** on the Library screen; the Kindle UI comes
   back. Or over SSH: `ssh kindle 'cd /mnt/us/eink-reader-dev && sh run-graphics.sh'`
   (`APP_ARGS=--log` for timings, `APP=./eink-reader-m0` for the test window).

The launcher pauses the Kindle UI processes only while the app runs and
resumes them on every exit path. Nothing autostarts at boot, and nothing
edits `/etc`, system partitions or KOReader.

## Runtime options

`eink-reader --help` lists them. Defaults: books from `/mnt/us/documents`
(read-only), state in `<exe dir>/data/state.json`, decoding in the
`render-worker` process when it sits next to the binary.

## Licence

Not chosen yet. The default plan in the spec is an open-source,
GPL-compatible licence: Slint's royalty-free licence excludes embedded
systems, so GPLv3 is the viable Slint path for this app. Review the full
dependency licence set (`cargo tree`) before distributing binaries. The font in
`assets/fonts/` is not committed; `scripts/fetch-font.sh` installs DejaVu Sans
with its licence.
