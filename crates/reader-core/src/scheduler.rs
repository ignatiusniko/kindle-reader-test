//! One background render worker with a capacity-1 "latest wins" slot
//! (spec §4, §24.6).
//!
//! * At most one expensive job runs at a time.
//! * Submitting a job replaces any job still waiting in the slot.
//! * Every job carries a generation id; when a newer job is submitted the
//!   running one is told to cancel (polled by the decoder) and its result is
//!   dropped, so a stale page never reaches the screen.
//! * A retryable failure (budget refusal, worker crash) is retried **once**
//!   at lower quality; there is no retry loop.
//! * The UI thread never blocks on decoding: results arrive through the
//!   `deliver` callback (the Slint app forwards them with
//!   `invoke_from_event_loop`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use image_pipeline::Quality;

use crate::render::{BookError, BookInfo, PageRenderer, PageRequest, PageResult};

#[derive(Debug, Clone, PartialEq)]
pub enum Job {
    Open { path: PathBuf, generation: u64 },
    Render(PageRequest),
}

impl Job {
    pub fn generation(&self) -> u64 {
        match self {
            Job::Open { generation, .. } => *generation,
            Job::Render(r) => r.generation,
        }
    }
}

#[derive(Debug)]
pub enum Event {
    Opened {
        generation: u64,
        path: PathBuf,
        result: Result<BookInfo, BookError>,
    },
    Page {
        generation: u64,
        page_index: u32,
        result: PageResult,
        /// Wall time from job start to result, including any retry.
        elapsed_ms: u64,
    },
}

struct Slot {
    job: Option<Job>,
    shutdown: bool,
}

struct Shared {
    slot: Mutex<Slot>,
    cv: Condvar,
    current: AtomicU64,
}

pub struct Scheduler {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl Scheduler {
    pub fn new(
        mut renderer: Box<dyn PageRenderer>,
        deliver: impl Fn(Event) + Send + 'static,
    ) -> Self {
        let shared = Arc::new(Shared {
            slot: Mutex::new(Slot {
                job: None,
                shutdown: false,
            }),
            cv: Condvar::new(),
            current: AtomicU64::new(0),
        });
        let s = shared.clone();
        let thread = std::thread::Builder::new()
            .name("render-scheduler".into())
            // Decoders are not recursive; a modest stack saves RAM.
            .stack_size(512 * 1024)
            .spawn(move || worker_loop(&s, renderer.as_mut(), &deliver))
            .expect("spawn render thread");
        Self {
            shared,
            thread: Some(thread),
        }
    }

    /// Replace whatever is waiting with `job` and mark it as current.
    pub fn submit(&self, job: Job) {
        let mut slot = self.shared.slot.lock().expect("scheduler poisoned");
        self.shared
            .current
            .store(job.generation(), Ordering::Release);
        slot.job = Some(job);
        self.shared.cv.notify_one();
    }

    /// Cancel everything: in-flight work is abandoned, results dropped.
    pub fn cancel_all(&self, generation: u64) {
        let mut slot = self.shared.slot.lock().expect("scheduler poisoned");
        self.shared.current.store(generation, Ordering::Release);
        slot.job = None;
    }
}

impl Drop for Scheduler {
    fn drop(&mut self) {
        {
            let mut slot = self.shared.slot.lock().expect("scheduler poisoned");
            slot.shutdown = true;
            slot.job = None;
            self.shared.current.store(u64::MAX, Ordering::Release);
            self.shared.cv.notify_one();
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn worker_loop(shared: &Shared, renderer: &mut dyn PageRenderer, deliver: &dyn Fn(Event)) {
    loop {
        let job = {
            let mut slot = shared.slot.lock().expect("scheduler poisoned");
            loop {
                if slot.shutdown {
                    return;
                }
                if let Some(job) = slot.job.take() {
                    break job;
                }
                slot = shared.cv.wait(slot).expect("scheduler poisoned");
            }
        };
        let generation = job.generation();
        let is_current = || shared.current.load(Ordering::Acquire) == generation;
        if !is_current() {
            continue;
        }
        match job {
            Job::Open { path, .. } => {
                let result = renderer.open(&path);
                if is_current() {
                    deliver(Event::Opened {
                        generation,
                        path,
                        result,
                    });
                }
            }
            Job::Render(req) => {
                let started = Instant::now();
                let cancelled = || !is_current();
                let mut result = renderer.render(&req, &cancelled);
                if result.is_retryable() && req.render.quality == Quality::Normal && is_current() {
                    let mut low = req.clone();
                    low.render.quality = Quality::LowMemory;
                    result = renderer.render(&low, &cancelled);
                    if let PageResult::Ready(p) = &mut result {
                        p.reduced_quality = true;
                    }
                }
                if is_current() && !matches!(result, PageResult::Cancelled) {
                    deliver(Event::Page {
                        generation,
                        page_index: req.page_index,
                        result,
                        elapsed_ms: started.elapsed().as_millis() as u64,
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{BookInfo, RenderedPage};
    use image_pipeline::{GrayImage, RenderRequest};
    use std::path::Path;
    use std::sync::mpsc;
    use std::time::Duration;

    /// Fake renderer: sleeps per page, fails on demand.
    struct Fake {
        delay: Duration,
        fail_normal: bool,
        calls: Arc<Mutex<Vec<(u32, Quality)>>>,
    }

    impl PageRenderer for Fake {
        fn open(&mut self, _: &Path) -> Result<BookInfo, BookError> {
            Ok(BookInfo {
                page_count: 10,
                skipped_entries: 0,
            })
        }
        fn render(&mut self, req: &PageRequest, cancelled: &dyn Fn() -> bool) -> PageResult {
            self.calls
                .lock()
                .unwrap()
                .push((req.page_index, req.render.quality));
            let end = Instant::now() + self.delay;
            while Instant::now() < end {
                if cancelled() {
                    return PageResult::Cancelled;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            if self.fail_normal && req.render.quality == Quality::Normal {
                return PageResult::TooLargeForBudget("fake".into());
            }
            if self.fail_normal {
                return PageResult::InternalWorkerFailure("still failing".into());
            }
            PageResult::Ready(RenderedPage {
                image: GrayImage::from_pixels(1, 1, vec![0]).unwrap(),
                generation: req.generation,
                page_index: req.page_index,
                decode_ms: 0,
                reduced_quality: false,
            })
        }
        fn kind(&self) -> &'static str {
            "fake"
        }
    }

    fn req(page: u32, generation: u64) -> Job {
        Job::Render(PageRequest {
            book_path: "/b.cbz".into(),
            page_index: page,
            render: RenderRequest::new(100, 100),
            generation,
        })
    }

    type Calls = Arc<Mutex<Vec<(u32, Quality)>>>;

    fn setup(delay_ms: u64, fail: bool) -> (Scheduler, mpsc::Receiver<Event>, Calls) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let s = Scheduler::new(
            Box::new(Fake {
                delay: Duration::from_millis(delay_ms),
                fail_normal: fail,
                calls: calls.clone(),
            }),
            move |e| {
                let _ = tx.lock().unwrap().send(e);
            },
        );
        (s, rx, calls)
    }

    #[test]
    fn latest_request_wins_and_stale_results_never_arrive() {
        let (s, rx, calls) = setup(50, false);
        for (i, page) in [1u32, 2, 3, 4, 5].iter().enumerate() {
            s.submit(req(*page, i as u64 + 1));
        }
        let ev = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let Event::Page {
            generation,
            page_index,
            result,
            ..
        } = ev
        else {
            panic!()
        };
        assert_eq!((generation, page_index), (5, 5));
        assert!(matches!(result, PageResult::Ready(_)));
        assert!(
            rx.recv_timeout(Duration::from_millis(200)).is_err(),
            "no stale results"
        );
        // Capacity 1: intermediate requests were mostly skipped.
        assert!(calls.lock().unwrap().len() <= 2);
    }

    #[test]
    fn retries_once_at_lower_quality_then_reports() {
        let (s, rx, calls) = setup(1, true);
        s.submit(req(3, 1));
        let Event::Page { result, .. } = rx.recv_timeout(Duration::from_secs(2)).unwrap() else {
            panic!()
        };
        assert!(matches!(result, PageResult::InternalWorkerFailure(_)));
        assert_eq!(
            *calls.lock().unwrap(),
            vec![(3, Quality::Normal), (3, Quality::LowMemory)]
        );
        assert!(
            rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "no retry loop"
        );
    }

    #[test]
    fn cancel_all_drops_in_flight_result() {
        let (s, rx, _) = setup(100, false);
        s.submit(req(1, 1));
        std::thread::sleep(Duration::from_millis(20));
        s.cancel_all(2);
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
        s.submit(Job::Open {
            path: "/x".into(),
            generation: 3,
        });
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            Event::Opened { generation: 3, .. }
        ));
    }

    #[test]
    fn drop_joins_cleanly_while_busy() {
        let (s, _rx, _) = setup(200, false);
        s.submit(req(1, 1));
        let t = Instant::now();
        drop(s);
        assert!(
            t.elapsed() < Duration::from_millis(150),
            "running job is cancelled on drop"
        );
    }
}
