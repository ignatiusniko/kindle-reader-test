# 0001 — Pure-Rust image codecs instead of libjpeg-turbo for v0.1

**Status:** accepted (revisit after G6 measurements)

The spec suggests `turbojpeg` for scaled JPEG decoding. We use
`jpeg-decoder` 0.3 (pure Rust, `scale()` gives DCT scaling 1/2, 1/4, 1/8)
and `png` 0.18 (row streaming).

- No native C library to cross-compile for ARMv7 musl; `cargo zigbuild`
  builds everything.
- `jpeg-decoder` supports the same IDCT scaling idea, and lets us cap its
  internal buffers (`set_max_decoding_buffer_size`).
- `png::Reader::next_row` lets non-interlaced PNGs stream into a box filter,
  so PNG memory is independent of image height — better than the spec's
  minimum ("reject or degrade oversized PNGs").

Trade-off: libjpeg-turbo is faster (NEON). If G6 shows cold page decode too
slow on the MT8113, swap the JPEG path behind `image-pipeline` without
touching callers.
