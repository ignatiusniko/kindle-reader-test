#!/bin/sh
# Render UI screenshots headlessly from synthetic books (host only).
#     scripts/snapshot.sh [OUT_DIR]
set -eu
cd "$(dirname "$0")/.."
OUT="${1:-dist/snapshots}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
cargo run -q -p test-fixtures --bin make-fixtures -- "$WORK/fx"
mkdir -p "$WORK/books"
cp "$WORK/fx/sample_12_pages.cbz" "$WORK/books/Sample_Manga_Vol_1.cbz"
cp "$WORK/fx/normal_png.cbz" "$WORK/books/PNG_Test_Book.cbz"
cp "$WORK/fx/rtl_order.cbz" "$WORK/books/Ordering_Test.cbz"
cargo run -q -p eink-reader -- snapshot "$OUT" --books "$WORK/books" --data "$WORK/data" --worker thread
