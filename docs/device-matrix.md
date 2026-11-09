# Device matrix

| Device | Codename | Target triple (proposed) | G0 | G1 | G2 | G3 | G4 | G5 | G6 | G7 |
|---|---|---|---|---|---|---|---|---|---|---|
| Kindle Basic 11th gen (2022) | KT5 / Cava / C2V2L3 | `armv7-unknown-linux-musleabihf` (verify in G0) | ☐ | ☐ | build ✅ / transfer ☐ | ☐ | ☐ | ☐ | ☐ | ☐ |
| Host (Linux x86_64, glibc) | — | native | n/a | n/a | n/a | n/a | headless UI ✅ | ✅ | ✅ | worker isolation ✅ |
| Host (x86_64 musl) | — | `x86_64-unknown-linux-musl` | n/a | n/a | n/a | n/a | n/a | n/a | MEM-01 ✅ (mimalloc) | n/a |
| qemu-arm user mode | — | `armv7-unknown-linux-musleabihf` | n/a | CLI ✅ | n/a | n/a | n/a | n/a | unit tests ✅ | spawn tests n/a¹ |

¹ qemu-user cannot exec a nested ARM child without binfmt_misc, so the
worker-spawn tests only run natively.

Notes on KT5 from public sources (to be confirmed by G0, never assumed):

- 6-inch, 300 ppi, 1072 × 1448 portrait; MediaTek MT8113; ~512 MiB RAM.
- `slint-backend-kindle` 0.7.1 contains an `MXCFB_SEND_UPDATE_MTK` path whose
  source comment mentions MediaTek devices including the Basic 2022. The
  project README does not list KT5 as tested. Treat as **unverified**.
- The backend grabs the touch device (`EVIOCGRAB`) and writes `/dev/fb0`
  directly; the Amazon UI must not be drawing at the same time (G3).
