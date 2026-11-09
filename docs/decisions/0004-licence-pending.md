# 0004 — Licence not yet selected

**Status:** open — decision for the project owner

Slint's royalty-free licence excludes embedded devices; its GPLv3 option
fits an open-source Kindle app. `slint-backend-kindle` is MIT/Apache-2.0.
mimalloc is MIT. Codecs and zip are MIT/Apache-2.0. DejaVu fonts are freely
redistributable. FBInk (not used) would be GPLv3+.

Recommendation from the spec: open-source under a GPL-compatible licence
(GPL-3.0-or-later is the simplest fit with Slint). Confirm with a full
`cargo tree`/licence audit before publishing binaries.
