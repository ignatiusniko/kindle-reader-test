# Fixtures

Generated, never committed (`*.cbz` is git-ignored). Create them with:

```bash
cargo run -p test-fixtures --bin make-fixtures -- tests/fixtures
```

Add `--stress` for the oversized JPEG/PNG files (host only; never generate
these on the Kindle). All images are drawn by `crates/test-fixtures`; do not
add copyrighted books here.
