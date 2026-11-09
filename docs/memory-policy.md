# Memory and rendering policy

## Budget (spec §5, §24.4)

At launch: `process_budget = min(64 MiB, 35% of MemAvailable)` (falls back
to `MemFree`, never adds page cache). Split: 60% decode scratch, 30% page
buffers, 10% slack. Prefetch is **off** (`allow_prefetch = false`).

In process mode the worker gets `--budget <decode_budget>` and, by default,
`RLIMIT_AS = decode_budget + 96 MiB` (code, stacks, allocator overhead —
**calibrate on device**). In thread mode one budget covers both.

These are starting caps, not measured safe values.

## Hard limits (`memory_budget::Limits::default()`)

| Limit | Default | Enforced in |
|---|---|---|
| ZIP central-directory entries | 20,000 | `cbz-reader` index |
| Readable pages | 10,000 | `cbz-reader` index |
| Uncompressed page entry | 32 MiB (declared **and** actual) | `cbz-reader::read_page` |
| Compression ratio (entries > 1 MiB) | 100:1 | `cbz-reader::check_entry` |
| Source dimension | 30,000 px | `image-pipeline` header check |
| Source pixels | 60 MP | `image-pipeline` header check |
| Output pixels | 1072 × 1448 × 4 | `image-pipeline::output_size`, IPC reply check |

All size arithmetic uses `memory_budget::required_bytes` (checked, 32-bit safe).

## Per-page planned peaks (reservations)

| Path | Reserved before allocating |
|---|---|
| Compressed entry | its uncompressed size |
| Output page | `out_w × out_h` (Gray8), held while the page lives |
| Baseline JPEG | `2 × scaled_w × scaled_h × bpp` (scaled by DCT 1/2…1/8) |
| Progressive JPEG | above **plus** full-resolution coefficients `w × h × bpp × 2` |
| Non-interlaced PNG | `~21 × width` bytes of row scratch (independent of height) |
| Interlaced PNG | full frame `w × h × 4` (refused if over budget) |
| Displayed page | `3 × out_w × out_h` for Slint's RGB copy |

Measured on host: 2400×3200 JPEG → planned peak < 16 MiB; 2000×8000 RGBA
PNG → < 1 MiB. A 1072×1448 Gray8 page is 1.48 MiB; Slint's RGB copy is
4.44 MiB. The Kindle backend also keeps its own full-screen RGB buffer
(≈ 4.4 MiB). **Peak RSS/PSS must be measured on the device.**

## Failure ladder

1. Budget refusal or worker death → retry once at half resolution per axis.
2. Still failing → error panel: *“This page needs too much memory. Try a
   lower quality preview or skip this page.”* with **Skip page** / **Back**.
3. Corrupt/unsupported page → message + **Skip page** / **Back**.
4. Three consecutive worker crashes → renderer disabled until the book is
   reopened (no restart loop).

## Allocator

musl's allocator showed unbounded RSS growth under MEM-01 on the host;
musl builds use mimalloc. See `decisions/0003-allocator-on-musl.md`.

## Display refresh

All screen updates go through the Kindle backend (one owner of the
framebuffer). UI uses black/white only; the manga page is grayscale and never
forced bilevel. No animations; the "Loading…" hint appears only after 400 ms,
the page counter disappears after 1.5 s. "Refresh screen" (Advanced) flashes
a full black frame for 250 ms to clear ghosting; the right cadence for
automatic full refreshes must be measured on KT5.
