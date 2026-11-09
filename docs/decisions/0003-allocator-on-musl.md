# 0003 — mimalloc as the global allocator on musl targets

**Status:** accepted — re-measure on KT5 in G6

MEM-01 on `x86_64-unknown-linux-musl` showed RSS growing from 18 MiB to
80 MiB over 4,000 page turns while the reservation budget returned to zero
every turn, and the same test on glibc stayed flat at ~10 MiB. See
`docs/test-results/2026-11-09-host.md`.

`eink-reader` and `render-worker` set `mimalloc::MiMalloc` as
`#[global_allocator]` under `cfg(target_env = "musl")`. With it the same
4,000-turn run stays at ~18–21 MiB. glibc builds are unchanged.

Costs: a slightly higher baseline RSS (~8 MiB above glibc on host), one C
dependency compiled by `cargo zigbuild`. Alternatives if mimalloc misbehaves
on the device: reuse page/scratch buffers across turns (buffer pool) or try
another allocator; MEM-01 decides.
