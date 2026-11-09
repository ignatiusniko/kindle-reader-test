# Device matrix

| Device | Codename | Target triple (proposed) | G0 | G1 | G2 | G3 | G4 | G5 | G6 | G7 |
|---|---|---|---|---|---|---|---|---|---|---|
| Kindle Basic 11th gen (2022), fw 5.18.5.0.1 | KT5 / Cava / C2V2L3 | `armv7-unknown-linux-musleabihf` | probe pending | ✅ | ✅ | ✅ manual | partial ✅¹ | partial ✅¹ | ☐ | ☐ |
| Host (Linux x86_64, glibc) | — | native | n/a | n/a | n/a | n/a | headless UI ✅ | ✅ | ✅ | worker isolation ✅ |
| Host (x86_64 musl) | — | `x86_64-unknown-linux-musl` | n/a | n/a | n/a | n/a | n/a | n/a | MEM-01 ✅ (mimalloc) | n/a |
| qemu-arm user mode | — | `armv7-unknown-linux-musleabihf` | n/a | CLI ✅ | n/a | n/a | n/a | n/a | unit tests ✅ | spawn tests n/a² |

¹ 2026-11-09, one device: hardware test window and reader launched via
the launcher, CBZ reading works and feels faster than KOReader (subjective).
Not yet run: 20-tap/corner/grey checks recorded, sleep/resume, MEM-01 on
device, 50× open/exit. See `docs/test-results/2026-11-09-kt5-first-run.md`.

² qemu-user cannot exec a nested ARM child without binfmt_misc, so the
worker-spawn tests only run natively.

Notes on KT5 from public sources (to be confirmed by G0, never assumed):

- 6-inch, 300 ppi, 1072 × 1448 portrait; MediaTek MT8113; ~512 MiB RAM.
- `slint-backend-kindle` 0.7.1 contains an `MXCFB_SEND_UPDATE_MTK` path whose
  source comment mentions MediaTek devices including the Basic 2022. The
  project README does not list KT5 as tested. Treat as **unverified**.
- The backend grabs the touch device (`EVIOCGRAB`) and writes `/dev/fb0`
  directly; the Amazon UI must not be drawing at the same time (G3).
