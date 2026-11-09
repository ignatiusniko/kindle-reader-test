//! MEM-01 (spec §26.4) on the host: 500 page turns must keep the budget
//! and process RSS bounded.
//!
//! This lives in its own test binary on purpose: RSS is per process, and
//! other workload tests (e.g. the huge-PNG decode) running in parallel in
//! the same process would inflate it.

// Same allocator as the musl release binaries (see decisions/0003).
#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::path::PathBuf;
use std::sync::{Mutex, mpsc};
use std::time::Duration;

use image_pipeline::RenderRequest;
use memory_budget::{Limits, MIB, MemoryBudget};
use reader_core::render::{InProcessRenderer, PageRequest, PageResult};
use reader_core::scheduler::{Event, Job, Scheduler};

fn write(tag: &str, bytes: Vec<u8>) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("eink-mem01-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(format!("{tag}.cbz"));
    std::fs::write(&p, bytes).unwrap();
    p
}

#[test]
fn mem01_500_turns_keep_budget_bounded() {
    let book = write("turns", test_fixtures::jpeg_book(8, 1000, 1500));
    let budget = MemoryBudget::new(32 * MIB);
    let r = InProcessRenderer::new(Limits::default(), budget.clone());
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let s = Scheduler::new(Box::new(r), move |e| {
        let _ = tx.lock().unwrap().send(e);
    });
    let mut peaks = Vec::new();
    let mut rss = Vec::new();
    let turns: u64 = std::env::var("EINK_MEM_TURNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(500);
    let sample_every = (turns / 10).max(1);
    for i in 0..turns {
        s.submit(Job::Render(PageRequest {
            book_path: book.clone(),
            page_index: (i % 8) as u32,
            render: RenderRequest::new(1072, 1448),
            generation: i + 1,
        }));
        let Event::Page { result, .. } = rx.recv_timeout(Duration::from_secs(10)).unwrap() else {
            panic!()
        };
        let PageResult::Ready(p) = result else {
            panic!("turn {i}: {result:?}")
        };
        drop(p); // UI converted it and released the buffer
        assert_eq!(budget.in_use(), 0, "turn {i}: reservations leaked");
        peaks.push(budget.peak());
        if i % sample_every == 0
            && let Some(st) = diagnostic_kit::ProcStatus::read_self()
        {
            rss.push(st.vm_rss_kib.unwrap_or(0));
        }
    }
    assert!(*peaks.last().unwrap() < 16 * MIB);
    // RSS must not grow steadily: last sample within 25% of the second one
    // (the first includes warm-up).
    eprintln!("rss samples (KiB): {rss:?}");
    if rss.len() > 3 {
        let (base, last) = (rss[1] as f64, *rss.last().unwrap() as f64);
        assert!(
            last < base * 1.25 + 4096.0,
            "RSS grew from {base} to {last} KiB: {rss:?}"
        );
    }
}
