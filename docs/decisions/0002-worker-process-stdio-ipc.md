# 0002 — Worker process over stdin/stdout pipes

**Status:** accepted

Milestone B needs decoding in a separate process. We use the worker's
stdin/stdout: one JSON line per message, then raw Gray8 bytes for a page.

- Works on any Linux kernel the Kindle may have; no shared memory or
  sockets to validate first.
- Exactly one display-sized copy crosses the boundary; the UI validates the
  size and reserves budget before reading.
- `RLIMIT_AS` in the worker turns runaway allocations into a worker abort
  instead of a system-wide OOM. It cannot protect against the kernel OOM
  killer choosing the UI.
- Fault injection (`--debug-abort-on-page`) exists only in debug builds.

Thread mode (`--worker thread`) stays available for measuring the cost of
the extra process on the device (spec §4 "Test whether process separation
costs too much RAM").
