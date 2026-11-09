# Architecture

```text
┌──────────────────────── eink-reader (UI process) ────────────────────────┐
│ Slint UI (ui/*.slint)  ── callbacks ──▶  app.rs (thread-local AppState)  │
│                                              │                           │
│                         reader_core::controller::Controller              │
│                  (state machine, navigator, tap gate, progress store)    │
│                                              │ Output::Submit(Job)       │
│                         reader_core::scheduler::Scheduler                │
│               1 worker thread · slot capacity 1 · generation ids         │
│                                              │                           │
│              PageRenderer ── InProcessRenderer (thread mode)             │
│                          └─ ProcessRenderer ──stdin/stdout──┐            │
└──────────────────────────────────────────────────────────────┼───────────┘
                                                               ▼
                       render-worker (separate process, RLIMIT_AS)
                       InProcessRenderer: cbz-reader → image-pipeline
```

## Data flow for one page turn

1. Tap → `Controller::tap(x)` → zone (30/40/30) → action by reading direction.
   The tap gate drops taps during a decode and within 300 ms of the last one.
2. `Controller` bumps the generation and emits `Submit(Job::Render)`.
3. `Scheduler::submit` replaces any waiting job and sets the current
   generation; a running decode sees `cancelled()` and stops early.
4. The renderer reads one ZIP entry (bounded), decodes it straight to a
   display-sized Gray8 buffer, holding a budget reservation for each buffer.
5. A retryable failure (budget refusal, worker death) is retried **once** at
   `Quality::LowMemory`; then the result is delivered.
6. The UI thread receives the event via `upgrade_in_event_loop`, drops it if
   the generation is stale, converts Gray8 → Slint image, and schedules a
   debounced atomic progress write.

## Module map (spec §4)

| Spec module | Code |
|---|---|
| `app` | `apps/kindle-reader/src/app.rs`, `reader_core::controller` |
| `ui` | `ui/*.slint` |
| `platform` | `apps/kindle-reader/src/platform.rs` (Kindle backend / desktop / headless) |
| `catalog` | `reader_core::catalog` |
| `cbz` | `crates/cbz-reader` |
| `decode`, `render` | `crates/image-pipeline` |
| `scheduler` | `reader_core::scheduler` |
| `cache` | none yet — prefetch/caching is off by default on KT5 (spec §24.6) |
| `persistence` | `crates/persistence` |
| `diagnostics` | `crates/diagnostic-kit`, Advanced panel, `--log` |

## Worker IPC

One JSON line per request/response; a `ready` response is followed by exactly
`width × height` Gray8 bytes. The UI checks the announced size against
`Limits::max_output_pixels` and reserves budget **before** reading, so the
payload is bounded and copied once. After 3 consecutive worker failures the
renderer stops respawning until the book is reopened (no restart loop).

## What the host build cannot tell us

E-Ink waveforms and ghosting, touch coordinate mapping, MTK EPDC ioctl
compatibility, Amazon UI hand-off, suspend/resume, and real memory headroom
with Amazon services running. Those are device gates G3–G7.
