# Testing

## Host (every change, CI)

```bash
scripts/check.sh
```

| Spec item (§26.3 / §26.4) | Test |
|---|---|
| `required_bytes` handles `u32::MAX` | `memory_budget::tests::required_bytes_handles_extremes` |
| 1,000+ entries index without decoding (CBZ-03) | `cbz-reader … indexing_many_entries_reads_metadata_only`, `host_workload::cbz03_…` |
| `../` names never escape | `cbz-reader … path_traversal_names_are_inert` |
| Invalid headers don't panic | `image-pipeline … corrupt_inputs_do_not_panic` |
| Budget refusal before large allocation | `progressive_jpeg_accounts_for_coefficients`, `interlaced_png_needs_full_frame_budget` |
| Latest request wins, no stale flash | `scheduler … latest_request_wins…`, `controller … stale_results_are_ignored` |
| Progress resumes after exit | `controller … first_open_then_resume_in_one_step`, UI flow test |
| Bad metadata never deletes files | `persistence … corrupt_state_is_moved_aside_not_deleted` |
| CBZ-01 12-page JPEG sample | `host_workload::cbz01_…` |
| MEM-01 500 turns, bounded | `tests/mem01.rs` (own process; set `EINK_MEM_TURNS=4000` for longer) |
| MEM-02 huge image | `host_workload::mem02_…` |
| ERR-01 corrupt page / bad zip / zip bomb | `host_workload::err01_…` |
| ISO-01 worker killed, retry once, UI alive | `render-worker/tests/isolation.rs` |
| UI: library → reader → options → fit width → errors → resume | `eink-reader … ui_flow_library_reader_options_errors` (headless Slint) |

ARM checks without a device (optional):

```bash
cargo zigbuild --tests --target armv7-unknown-linux-musleabihf -p reader-core
```

Run the produced test executables with `qemu-arm`. RSS under qemu includes
the emulator and is not a valid memory measurement.

## Device (in gate order, never skip)

See spec §26.4 for the full table (HW-01 … REL-01) and
`docs/safety-and-recovery.md` for the gate rules. Record each run in
`docs/test-results/` with the template below. Never fill missing
measurements with guesses.

```yaml
test_run:
  date: null
  device: "Kindle Basic 11th Gen (2022)"
  firmware: null
  git_revision: null
  build_target: null
  launcher_version: null
  test_id: null
  book_fixture: null
  page_count: null
  run_count: null
  display_mode: "grayscale"
  mem_available_start_kib: null
  ui_rss_peak_kib: null
  worker_rss_peak_kib: null
  combined_pss_peak_kib: null
  render_p50_ms: null
  render_p95_ms: null
  display_submit_p95_ms: null
  errors: 0
  recoveries: 0
  passed: false
  notes: ""
```

`--log` writes per-page `decode_ms` and `tap_to_present_ms` to
`<data>/logs/reader.log` (512 KiB cap, no file names) for P50/P95.
