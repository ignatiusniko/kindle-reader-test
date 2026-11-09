//! M3 / ISO-01 on the host: the UI-side renderer survives worker death.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, mpsc};
use std::time::Duration;

use image_pipeline::RenderRequest;
use memory_budget::{Limits, MIB, MemoryBudget};
use reader_core::protocol::ProcessRenderer;
use reader_core::render::{BookErrorKind, PageRenderer, PageRequest, PageResult};
use reader_core::scheduler::{Event, Job, Scheduler};

fn worker_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_render-worker"))
}

fn fixture(tag: &str, bytes: Vec<u8>) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("eink-iso-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(format!("{tag}.cbz"));
    std::fs::write(&p, bytes).unwrap();
    p
}

fn renderer(args: &[&str]) -> ProcessRenderer {
    ProcessRenderer::new(
        worker_exe(),
        args.iter().map(|s| s.to_string()).collect(),
        Limits::default(),
        MemoryBudget::new(32 * MIB),
    )
}

fn req(path: &Path, page: u32) -> PageRequest {
    PageRequest {
        book_path: path.to_owned(),
        page_index: page,
        render: RenderRequest::new(1072, 1448),
        generation: 1,
    }
}

#[test]
fn renders_pages_over_ipc() {
    let book = fixture("ipc", test_fixtures::jpeg_book(3, 1200, 1800));
    let mut r = renderer(&[]);
    assert_eq!(r.open(&book).unwrap().page_count, 3);
    let PageResult::Ready(p) = r.render(&req(&book, 2), &|| false) else {
        panic!()
    };
    assert_eq!((p.image.width, p.image.height), (965, 1448));
    // Page border is black, centre-left white.
    assert_eq!(p.image.pixels.len(), 965 * 1448);
    let missing = r.open(Path::new("/nonexistent/x.cbz")).unwrap_err();
    assert_eq!(missing.kind, BookErrorKind::NotFound);
}

#[test]
fn killed_worker_is_reported_and_respawned() {
    let book = fixture("kill", test_fixtures::jpeg_book(3, 600, 900));
    let mut r = renderer(&[]);
    r.open(&book).unwrap();
    let pid = r.ping().unwrap();
    // SAFETY: killing our own child process.
    unsafe { libc_kill(pid) };
    std::thread::sleep(Duration::from_millis(50));
    let res = r.render(&req(&book, 0), &|| false);
    assert!(
        matches!(res, PageResult::InternalWorkerFailure(_)),
        "{res:?}"
    );
    // Next request transparently spawns a fresh worker.
    assert!(matches!(
        r.render(&req(&book, 1), &|| false),
        PageResult::Ready(_)
    ));
    assert_ne!(r.ping().unwrap(), pid);
}

unsafe fn libc_kill(pid: u32) {
    unsafe extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    unsafe { kill(pid as i32, 9) };
}

#[test]
fn crash_on_page_retries_once_then_shows_error_and_ui_survives() {
    let book = fixture("crash", test_fixtures::jpeg_book(4, 600, 900));
    let r = renderer(&["--debug-abort-on-page", "2"]);
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let s = Scheduler::new(Box::new(r), move |e| {
        let _ = tx.lock().unwrap().send(e);
    });
    s.submit(Job::Render(req(&book, 2)));
    let Event::Page { result, .. } = rx.recv_timeout(Duration::from_secs(10)).unwrap() else {
        panic!()
    };
    assert!(
        matches!(result, PageResult::InternalWorkerFailure(_)),
        "{result:?}"
    );
    // The "UI" (this test process) is alive and other pages still render.
    let mut good = req(&book, 1);
    good.generation = 2;
    s.submit(Job::Render(good));
    let Event::Page { result, .. } = rx.recv_timeout(Duration::from_secs(10)).unwrap() else {
        panic!()
    };
    assert!(matches!(result, PageResult::Ready(_)), "{result:?}");
}

#[test]
fn address_space_limit_contains_allocation_failure() {
    // Progressive 5000×5000 JPEG needs ~50 MB of coefficients. The budget
    // is deliberately huge so only RLIMIT_AS (40 MB) stops it: the worker
    // must fail on its own, and the caller must stay usable.
    let px = vec![200u8; 5000 * 5000];
    let jpg = test_fixtures::encode_jpeg(&px, 5000, 5000, true);
    let book = fixture(
        "rlimit",
        test_fixtures::zip(&[("1.jpg".into(), jpg, false)]),
    );
    let mut r = renderer(&[
        "--budget",
        "1000000000",
        "--address-space-limit",
        "41943040",
    ]);
    let res = r.render(&req(&book, 0), &|| false);
    assert!(
        matches!(
            res,
            PageResult::InternalWorkerFailure(_) | PageResult::TooLargeForBudget(_)
        ),
        "{res:?}"
    );
    // Caller alive; a fresh worker answers.
    assert!(r.ping().is_ok());
}

#[test]
fn corrupt_page_is_an_error_not_a_crash() {
    let book = fixture("corrupt", test_fixtures::corrupt_page_book(4, 2));
    let mut r = renderer(&[]);
    assert!(matches!(
        r.render(&req(&book, 1), &|| false),
        PageResult::CorruptPage(_)
    ));
    assert!(matches!(
        r.render(&req(&book, 2), &|| false),
        PageResult::Ready(_)
    ));
}

#[test]
fn hung_worker_times_out_and_is_replaced() {
    let book = fixture("hang", test_fixtures::jpeg_book(3, 600, 900));
    let mut r = renderer(&["--debug-hang-on-page", "1"])
        .with_timeouts(Duration::from_millis(500), Duration::from_secs(5));
    let pid = r.ping().unwrap();
    let t = std::time::Instant::now();
    let res = r.render(&req(&book, 1), &|| false);
    assert!(
        matches!(res, PageResult::InternalWorkerFailure(_)),
        "{res:?}"
    );
    assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    // The stuck worker was killed and a new one answers.
    assert!(matches!(
        r.render(&req(&book, 0), &|| false),
        PageResult::Ready(_)
    ));
    assert_ne!(r.ping().unwrap(), pid);
}

#[test]
fn cancelling_a_stuck_render_returns_promptly() {
    let book = fixture("cancel", test_fixtures::jpeg_book(3, 600, 900));
    let mut r = renderer(&["--debug-hang-on-page", "2"]);
    let t = std::time::Instant::now();
    let cancel_after = t + Duration::from_millis(200);
    let res = r.render(&req(&book, 2), &|| std::time::Instant::now() > cancel_after);
    assert!(matches!(res, PageResult::Cancelled), "{res:?}");
    assert!(t.elapsed() < Duration::from_secs(2));
    // Cancellation is not counted as a crash; the next page renders.
    assert!(matches!(
        r.render(&req(&book, 0), &|| false),
        PageResult::Ready(_)
    ));
}

#[test]
fn scheduler_drop_does_not_hang_on_stuck_worker() {
    let book = fixture("drop", test_fixtures::jpeg_book(3, 600, 900));
    let r = renderer(&["--debug-hang-on-page", "0"]);
    let s = Scheduler::new(Box::new(r), |_| {});
    s.submit(Job::Render(req(&book, 0)));
    std::thread::sleep(Duration::from_millis(300));
    let t = std::time::Instant::now();
    drop(s);
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
}
