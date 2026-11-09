# E-Ink Reader (experimental) — Kindle Basic 2022

A minimal, offline CBZ manga reader for E-Ink Kindles, written in Rust with a
Slint software-rendered UI. Built from *E-Ink Reader — Technical Spec v0.3*.

> **Status: experimental. Nothing here has run on a Kindle Basic 2022
> (KT5 / Cava / C2V2L3) yet.** The host-side reader, decoder, memory policy,
> worker isolation and UI are implemented and tested on a development
> machine, and every binary cross-compiles to a static ARMv7 musl ELF. The
> device gates G0–G7 still have to be run on real hardware, in order. Do not
> launch a graphical binary on the Kindle until gate G3 (display hand-off and
> restore) has been established for *your* firmware.

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
| Display, touch, refresh, sleep/resume, Kindle UI hand-off on KT5 | ❌ **not tested — device gates G0–G7** |

## Repository layout

```text
apps/cli-probe        G1 smoke test (no display)
apps/kindle-hwtest    G2/G4 Slint test window: tap counter + exit (eink-reader-m0)
apps/kindle-reader    The reader UI (eink-reader); features: kindle | desktop
apps/render-worker    Isolated decode process (M3)
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
7. **Stop.** Graphics need G3: write and rehearse `handoff_display` /
   `restore_display` in `run-graphics.sh.template` and fill in
   `scripts/sample-recovery-checklist.md`. See `docs/safety-and-recovery.md`.

The project deliberately ships **no** command that stops Amazon services,
remounts partitions, edits `/etc` or autostarts at boot.

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
