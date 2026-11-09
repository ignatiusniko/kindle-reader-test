# Build environment

Recorded for the first successful build (2026-11-09, Linux x86_64 host).

| Tool / crate | Version |
|---|---|
| rustc / cargo | 1.97.0 |
| Zig (via `pip install ziglang`) | 0.17.1 |
| cargo-zigbuild | 0.24.1 |
| slint / slint-build | 1.18.1 (`default-features = false`, `compat-1-2`, `std`, `renderer-software`) |
| slint-backend-kindle | 0.7.1 |
| zip | 9.5.0 (deflate only, pure-Rust miniz_oxide backend) |
| jpeg-decoder | 0.3.2 (no rayon) |
| png | 0.18.1 |
| mimalloc | 0.1.52 (musl builds only) |

Exact versions of everything are pinned in `Cargo.lock` (committed).

Cross-build:

```bash
rustup target add armv7-unknown-linux-musleabihf
```

```bash
cargo install cargo-zigbuild
```

```bash
scripts/build-kindle.sh
```

Result (release, stripped, static): `eink-reader` ≈ 7.9 MB,
`eink-reader-m0` ≈ 6.9 MB, `render-worker` ≈ 0.9 MB, `eink-reader-cli-probe`
≈ 0.3 MB; all `ELF 32-bit LSB executable, ARM, EABI5, statically linked`.

No native C libraries are needed except mimalloc, which is compiled by
`cargo zigbuild` along with everything else.
