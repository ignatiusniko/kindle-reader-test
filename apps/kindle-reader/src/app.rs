//! Glue between the Slint UI and `reader_core::controller::Controller`.
//!
//! All state lives on the UI thread in a thread-local. Background work (page
//! decoding, folder scans) reports back with `upgrade_in_event_loop`, so the
//! UI thread never blocks on I/O or decoding.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use diagnostic_kit::{BoundedLog, MemInfo, ProcStatus};
use image_pipeline::GrayImage;
use memory_budget::{BudgetPlan, KIB, Limits, MemoryBudget, Reservation};
use persistence::{ContrastPref, Direction, Fit, LoadOutcome, ReadingPrefs, Store};
use reader_core::catalog::{self, CatalogEntry};
use reader_core::controller::{Controller, LibraryModel, Output, Viewport};
use reader_core::protocol::ProcessRenderer;
use reader_core::render::{InProcessRenderer, PageRenderer};
use reader_core::scheduler::{Event, Job, Scheduler};
use slint::{ComponentHandle, Model, ModelRc, SharedPixelBuffer, SharedString, VecModel};

use crate::config::{Config, WorkerMode};
use crate::{AppWindow, BookItem, Theme};

/// Where render jobs go. The real app uses the threaded [`Scheduler`];
/// snapshots and UI tests use [`SyncSink`] so they run without an event loop.
pub trait JobSink {
    fn submit(&self, job: Job);
    fn cancel_all(&self, generation: u64);
}

impl JobSink for Scheduler {
    fn submit(&self, job: Job) {
        Scheduler::submit(self, job)
    }
    fn cancel_all(&self, generation: u64) {
        Scheduler::cancel_all(self, generation)
    }
}

/// Runs jobs immediately on the calling thread and queues the events.
pub struct SyncSink {
    pub renderer: RefCell<Box<dyn PageRenderer>>,
    pub events: Rc<RefCell<VecDeque<Event>>>,
}

impl JobSink for SyncSink {
    fn submit(&self, job: Job) {
        let mut r = self.renderer.borrow_mut();
        let ev = match job {
            Job::Open { path, generation } => Event::Opened {
                generation,
                result: r.open(&path),
                path,
            },
            Job::Render(req) => Event::Page {
                generation: req.generation,
                page_index: req.page_index,
                result: r.render(&req, &|| false),
                elapsed_ms: 0,
            },
        };
        self.events.borrow_mut().push_back(ev);
    }
    fn cancel_all(&self, _generation: u64) {}
}

const DEFAULT_VIEWPORT: Viewport = Viewport {
    width: 1072,
    height: 1448,
};

pub struct AppState {
    weak: slint::Weak<AppWindow>,
    ctl: Controller,
    sink: Box<dyn JobSink>,
    cfg: Config,
    catalog: Option<Vec<CatalogEntry>>,
    list_page: usize,
    budget: MemoryBudget,
    renderer_kind: &'static str,
    log: BoundedLog,
    displayed: Option<Reservation>,
    loading: bool,
    last_decode_ms: u64,
    last_reduced: bool,
    tap_at: Option<Instant>,
    persist_timer: slint::Timer,
    loading_timer: slint::Timer,
    notice_timer: slint::Timer,
    progress_timer: slint::Timer,
    flash_timer: slint::Timer,
}

thread_local! {
    static STATE: RefCell<Option<AppState>> = const { RefCell::new(None) };
}

/// Run `f` with the UI state, if it exists.
pub fn with_state<R>(f: impl FnOnce(&mut AppState) -> R) -> Option<R> {
    STATE.with(|s| s.borrow_mut().as_mut().map(f))
}

pub fn budget_plan() -> BudgetPlan {
    match MemInfo::read().ok().and_then(|m| m.effective_available()) {
        Some(avail) => BudgetPlan::from_available(avail),
        None => BudgetPlan::conservative_default(),
    }
}

/// Build the renderer chosen in the config. Returns it with the budget the
/// UI side should account page buffers against.
pub fn make_renderer(cfg: &Config, plan: &BudgetPlan) -> (Box<dyn PageRenderer>, MemoryBudget) {
    let limits = Limits::default();
    match cfg.worker {
        WorkerMode::Thread => {
            let budget = MemoryBudget::new(plan.decode_budget + plan.page_buffer_budget);
            (
                Box::new(InProcessRenderer::new(limits, budget.clone())),
                budget,
            )
        }
        WorkerMode::Process => {
            let as_limit = if cfg.worker_as_limit == usize::MAX {
                // Default: decode budget + generous slack for code, stacks
                // and allocator overhead. Calibrate on device (spec §12 M2).
                plan.decode_budget + 96 * memory_budget::MIB
            } else {
                cfg.worker_as_limit
            };
            let mut args = vec!["--budget".to_string(), plan.decode_budget.to_string()];
            if as_limit > 0 {
                args.push("--address-space-limit".into());
                args.push(as_limit.to_string());
            }
            let budget = MemoryBudget::new(plan.page_buffer_budget);
            (
                Box::new(ProcessRenderer::new(
                    cfg.worker_exe.clone(),
                    args,
                    limits,
                    budget.clone(),
                )),
                budget,
            )
        }
    }
}

fn mm_px(dpi: f32) -> f32 {
    dpi / 25.4
}

fn gray_to_image(img: &GrayImage) -> slint::Image {
    let mut buf = SharedPixelBuffer::<slint::Rgb8Pixel>::new(img.width, img.height);
    for (d, s) in buf.make_mut_slice().iter_mut().zip(&img.pixels) {
        *d = slint::Rgb8Pixel {
            r: *s,
            g: *s,
            b: *s,
        };
    }
    slint::Image::from_rgb8(buf)
}

fn book_item(i: &reader_core::controller::LibraryItem) -> BookItem {
    BookItem {
        id: i.id.as_str().into(),
        title: i.title.as_str().into(),
        progress: i.progress_text().into(),
        missing: i.missing,
    }
}

/// Create the state and wire every callback. `sink` decides how jobs run.
pub fn install(
    app: &AppWindow,
    cfg: Config,
    sink_factory: impl FnOnce(&AppWindow, Box<dyn PageRenderer>) -> Box<dyn JobSink>,
) -> Result<(), String> {
    let plan = budget_plan();
    let (renderer, budget) = make_renderer(&cfg, &plan);
    let renderer_kind = renderer.kind();
    let (store, outcome) = Store::open(&cfg.data_dir)
        .map_err(|e| format!("cannot open data folder {}: {e}", cfg.data_dir.display()))?;
    let mut log = if cfg.log {
        let dir = cfg.data_dir.join("logs");
        let _ = std::fs::create_dir_all(&dir);
        BoundedLog::open(dir.join("reader.log"), 512 * 1024)
    } else {
        BoundedLog::disabled()
    };
    log.line(&format!(
        "start renderer={renderer_kind} process_budget_kib={} decode_kib={} page_kib={}",
        plan.process_budget / KIB,
        plan.decode_budget / KIB,
        plan.page_buffer_budget / KIB
    ));
    if let LoadOutcome::RecoveredFromCorrupt(p) = &outcome {
        log.line(&format!(
            "state file was unreadable; moved to {}",
            p.display()
        ));
    }
    app.global::<Theme>().set_mm(mm_px(cfg.dpi));
    let viewport = cfg
        .viewport
        .map(|(width, height)| Viewport { width, height })
        .unwrap_or(DEFAULT_VIEWPORT);
    let sink = sink_factory(app, renderer);
    let state = AppState {
        weak: app.as_weak(),
        ctl: Controller::new(store, viewport),
        sink,
        cfg,
        catalog: None,
        list_page: 0,
        budget,
        renderer_kind,
        log,
        displayed: None,
        loading: false,
        last_decode_ms: 0,
        last_reduced: false,
        tap_at: None,
        persist_timer: slint::Timer::default(),
        loading_timer: slint::Timer::default(),
        notice_timer: slint::Timer::default(),
        progress_timer: slint::Timer::default(),
        flash_timer: slint::Timer::default(),
    };
    STATE.with(|s| *s.borrow_mut() = Some(state));
    wire_callbacks(app);
    with_state(|st| st.refresh_library());
    Ok(())
}

/// Threaded scheduler that posts results back to the UI thread.
pub fn threaded_sink(app: &AppWindow, renderer: Box<dyn PageRenderer>) -> Box<dyn JobSink> {
    let weak = app.as_weak();
    Box::new(Scheduler::new(renderer, move |ev| {
        let _ = weak.upgrade_in_event_loop(move |_| {
            with_state(|st| st.on_scheduler_event(ev));
        });
    }))
}

fn wire_callbacks(app: &AppWindow) {
    app.on_open_book(|id| {
        with_state(|st| st.open_book(&id));
    });
    app.on_list_prev(|| {
        with_state(|st| {
            st.list_page = st.list_page.saturating_sub(1);
            st.refresh_library();
        });
    });
    app.on_list_next(|| {
        with_state(|st| {
            st.list_page += 1;
            st.refresh_library();
        });
    });
    app.on_rescan(|| {
        with_state(|st| st.start_scan());
    });
    app.on_exit_app(|| {
        with_state(|st| st.flush());
        let _ = slint::quit_event_loop();
    });
    app.on_page_tap(|x| {
        with_state(|st| {
            st.tap_at = Some(Instant::now());
            let out = st.ctl.tap(x, Instant::now());
            st.apply(out);
        });
    });
    app.on_set_fit_width(|w| {
        with_state(|st| {
            let out = st.ctl.set_fit(if w { Fit::Width } else { Fit::Page });
            st.apply(out);
        });
    });
    app.on_set_rtl(|rtl| {
        with_state(|st| {
            let d = if rtl {
                Direction::RightToLeft
            } else {
                Direction::LeftToRight
            };
            let out = st.ctl.set_direction(d);
            st.apply(out);
        });
    });
    app.on_prev_page(|| {
        with_state(|st| {
            let out = st.ctl.previous_page();
            st.apply(out);
        });
    });
    app.on_next_page(|| {
        with_state(|st| {
            let out = st.ctl.next_page();
            st.apply(out);
        });
    });
    app.on_jump(|f| {
        with_state(|st| {
            let out = st.ctl.jump_fraction(f);
            st.apply(out);
        });
    });
    app.on_set_darker(|d| {
        with_state(|st| {
            let c = if d {
                ContrastPref::Darker
            } else {
                ContrastPref::Normal
            };
            let out = st.ctl.set_contrast(c);
            st.apply(out);
        });
    });
    app.on_set_show_progress(|on| {
        with_state(|st| {
            let out = st.ctl.set_show_progress(on);
            st.apply(out);
        });
    });
    app.on_refresh_screen(|| {
        with_state(|st| st.flash());
    });
    app.on_to_library(|| {
        with_state(|st| {
            let out = st.ctl.close();
            st.apply(out);
        });
    });
    app.on_retry(|| {
        with_state(|st| {
            let out = st.ctl.retry();
            st.apply(out);
        });
    });
    app.on_skip(|| {
        with_state(|st| {
            let out = st.ctl.skip_page();
            st.apply(out);
        });
    });
}

impl AppState {
    fn app(&self) -> Option<AppWindow> {
        self.weak.upgrade()
    }

    pub fn start_scan(&mut self) {
        let Some(app) = self.app() else { return };
        app.set_library_status("Looking for books…".into());
        let dir = self.cfg.books_dir.clone();
        let weak = self.weak.clone();
        let spawn = std::thread::Builder::new()
            .name("catalog-scan".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                let result = catalog::scan(&dir, 2, 5000);
                let _ = weak.upgrade_in_event_loop(move |_| {
                    with_state(|st| st.on_catalog(result));
                });
            });
        if spawn.is_err() {
            app.set_library_status("Could not scan the book folder.".into());
        }
    }

    /// Synchronous scan (snapshots/tests).
    pub fn scan_now(&mut self) {
        let r = catalog::scan(&self.cfg.books_dir, 2, 5000);
        self.on_catalog(r);
    }

    pub fn on_catalog(&mut self, result: std::io::Result<Vec<CatalogEntry>>) {
        match result {
            Ok(entries) => {
                self.ctl.sync_catalog(&entries);
                self.catalog = Some(entries);
                self.schedule_persist();
            }
            Err(e) => {
                self.log.line(&format!("scan failed: {}", e.kind()));
                self.catalog = Some(Vec::new());
                self.ctl.sync_catalog(&[]);
            }
        }
        self.refresh_library();
    }

    fn rows_per_page(&self, app: &AppWindow, has_continue: bool) -> usize {
        let mm = mm_px(self.cfg.dpi);
        let tap = 11.0 * mm;
        let h = app.window().size().height as f32 / app.window().scale_factor();
        let h = if h > 0.0 {
            h
        } else {
            self.ctl.viewport().height as f32
        };
        let fixed = 2.0 * 4.0 * mm      // padding
            + 2.0 * tap                 // header + footer buttons
            + 6.0 * 2.5 * mm            // spacing
            + 3.0 * mm * 1.6            // "Books" label
            + if has_continue { 2.6 * tap + 2.5 * mm } else { 0.0 };
        (((h - fixed) / (1.25 * tap)).floor() as usize).max(1)
    }

    pub fn refresh_library(&mut self) {
        let Some(app) = self.app() else { return };
        let LibraryModel {
            continue_reading,
            books,
        } = self.ctl.library();
        app.set_has_continue(continue_reading.is_some());
        if let Some(c) = &continue_reading {
            app.set_continue_book(book_item(c));
        }
        let per = self.rows_per_page(&app, continue_reading.is_some());
        let pages = books.len().div_ceil(per).max(1);
        self.list_page = self.list_page.min(pages - 1);
        let slice: Vec<BookItem> = books
            .iter()
            .skip(self.list_page * per)
            .take(per)
            .map(book_item)
            .collect();
        app.set_books(ModelRc::new(VecModel::from(slice)));
        app.set_list_page(self.list_page as i32);
        app.set_list_pages(pages as i32);
        let status = match &self.catalog {
            None => "Looking for books…".to_string(),
            Some(_) if books.is_empty() => format!(
                "No .cbz books found in {}.\nCopy CBZ files there, then tap Rescan.",
                self.cfg.books_dir.display()
            ),
            Some(_) => String::new(),
        };
        app.set_library_status(status.into());
    }

    fn update_viewport(&mut self, app: &AppWindow) {
        if self.cfg.viewport.is_some() {
            return;
        }
        let s = app.window().size();
        if s.width > 0 && s.height > 0 {
            self.ctl.set_viewport(Viewport {
                width: s.width,
                height: s.height,
            });
        }
    }

    pub fn open_book(&mut self, id: &str) {
        if let Some(app) = self.app() {
            self.update_viewport(&app);
        }
        let out = self.ctl.open(id);
        self.apply(out);
    }

    pub fn on_scheduler_event(&mut self, ev: Event) {
        let out = self.ctl.on_event(ev);
        self.apply(out);
    }

    pub fn flush(&mut self) {
        if let Err(e) = self.ctl.store_mut().flush() {
            self.log
                .line(&format!("progress save failed: {}", e.kind()));
        }
    }

    fn schedule_persist(&mut self) {
        if self.persist_timer.running() {
            return;
        }
        let delay = self.ctl.store().debounce();
        self.persist_timer
            .start(slint::TimerMode::SingleShot, delay, || {
                with_state(|st| st.flush());
            });
    }

    fn flash(&mut self) {
        let Some(app) = self.app() else { return };
        app.set_flash(true);
        let weak = self.weak.clone();
        self.flash_timer.start(
            slint::TimerMode::SingleShot,
            Duration::from_millis(250),
            move || {
                if let Some(a) = weak.upgrade() {
                    a.set_flash(false);
                }
            },
        );
    }

    fn set_prefs(app: &AppWindow, p: ReadingPrefs) {
        app.set_fit_width(p.fit == Fit::Width);
        app.set_rtl(p.direction == Direction::RightToLeft);
        app.set_darker(p.contrast == ContrastPref::Darker);
        app.set_show_progress(p.show_progress);
    }

    fn diagnostics(&self) -> String {
        let rss = ProcStatus::read_self()
            .and_then(|s| s.vm_rss_kib)
            .map(|k| format!("{k} KiB"))
            .unwrap_or_else(|| "n/a".into());
        let avail = MemInfo::read()
            .ok()
            .and_then(|m| m.effective_available())
            .map(|b| format!("{} KiB", b / KIB))
            .unwrap_or_else(|| "n/a".into());
        format!(
            "Renderer: {} · last page {} ms{}\nPage buffers: {} KiB in use, peak {} of {} KiB\nUI RSS: {} · MemAvailable: {}",
            self.renderer_kind,
            self.last_decode_ms,
            if self.last_reduced {
                " (reduced quality)"
            } else {
                ""
            },
            self.budget.in_use() / KIB,
            self.budget.peak() / KIB,
            self.budget.limit() / KIB,
            rss,
            avail
        )
    }

    pub fn apply(&mut self, outputs: Vec<Output>) {
        let Some(app) = self.app() else { return };
        for o in outputs {
            match o {
                Output::Submit(job) => self.sink.submit(job),
                Output::CancelAll(g) => self.sink.cancel_all(g),
                Output::EnterReader { title } => {
                    app.set_book_title(title.into());
                    app.set_has_image(false);
                    app.set_page_image(slint::Image::default());
                    self.displayed = None;
                    app.set_error_visible(false);
                    app.set_options_open(false);
                    app.set_advanced_open(false);
                    app.set_page_offset(0.0);
                    app.set_screen(1);
                }
                Output::Loading { .. } => {
                    self.loading = true;
                    let weak = self.weak.clone();
                    // Only show a hint for slow pages: every extra element
                    // costs an E-Ink refresh.
                    self.loading_timer.start(
                        slint::TimerMode::SingleShot,
                        Duration::from_millis(400),
                        move || {
                            let still = with_state(|st| st.loading).unwrap_or(false);
                            if let Some(a) = weak.upgrade() {
                                a.set_loading_visible(still);
                            }
                        },
                    );
                }
                Output::Present {
                    image,
                    page,
                    page_count,
                    offset_px,
                    display_height,
                    reduced_quality,
                    decode_ms,
                } => {
                    self.loading = false;
                    self.loading_timer.stop();
                    // Account for the RGB copy Slint keeps for display.
                    self.displayed = None;
                    self.displayed = self
                        .budget
                        .reserve(image.pixels.len() * 3, "displayed page")
                        .ok();
                    let img = gray_to_image(&image);
                    drop(image);
                    app.set_page_image(img);
                    app.set_has_image(true);
                    app.set_loading_visible(false);
                    app.set_error_visible(false);
                    app.set_page_number(page as i32 + 1);
                    app.set_page_count(page_count as i32);
                    app.set_page_display_height(display_height);
                    app.set_page_offset(offset_px);
                    self.last_decode_ms = decode_ms;
                    self.last_reduced = reduced_quality;
                    let tap_ms = self.tap_at.take().map(|t| t.elapsed().as_millis());
                    self.log.line(&format!(
                        "page decode_ms={decode_ms} tap_to_present_ms={} reduced={reduced_quality} budget_peak_kib={}",
                        tap_ms.map(|v| v.to_string()).unwrap_or_else(|| "-".into()),
                        self.budget.peak() / KIB
                    ));
                    app.set_progress_visible(true);
                    let weak = self.weak.clone();
                    self.progress_timer.start(
                        slint::TimerMode::SingleShot,
                        Duration::from_millis(1500),
                        move || {
                            if let Some(a) = weak.upgrade() {
                                a.set_progress_visible(false);
                            }
                        },
                    );
                    if app.get_options_open() {
                        app.set_diagnostics(self.diagnostics().into());
                    }
                }
                Output::Scroll { offset_px } => app.set_page_offset(offset_px),
                Output::ShowError(v) => {
                    self.loading = false;
                    self.loading_timer.stop();
                    app.set_loading_visible(false);
                    app.set_options_open(false);
                    app.set_error_message(v.message.into());
                    app.set_error_can_skip(v.can_skip);
                    app.set_error_can_retry(v.can_retry);
                    app.set_error_visible(true);
                    self.log.line("page error shown");
                }
                Output::ShowOptions(open) => {
                    app.set_diagnostics(self.diagnostics().into());
                    app.set_options_open(open);
                }
                Output::Prefs(p) => Self::set_prefs(&app, p),
                Output::Notice(text) => {
                    app.set_notice(SharedString::from(text));
                    let weak = self.weak.clone();
                    self.notice_timer.start(
                        slint::TimerMode::SingleShot,
                        Duration::from_millis(1500),
                        move || {
                            if let Some(a) = weak.upgrade() {
                                a.set_notice(SharedString::new());
                            }
                        },
                    );
                }
                Output::Persist => self.schedule_persist(),
                Output::ToLibrary => {
                    self.loading = false;
                    app.set_page_image(slint::Image::default());
                    self.displayed = None;
                    app.set_has_image(false);
                    app.set_error_visible(false);
                    app.set_options_open(false);
                    app.set_screen(0);
                    self.refresh_library();
                }
            }
        }
    }
}

/// Flush state and drop the scheduler (joins the render thread).
pub fn shutdown() {
    let state = STATE.with(|s| s.borrow_mut().take());
    if let Some(mut st) = state {
        st.flush();
        st.log.line("exit");
        drop(st);
    }
}

/// Helper for tests/snapshots: first catalog book id, if any.
pub fn first_book_id() -> Option<String> {
    with_state(|st| st.ctl.library().books.first().map(|b| b.id.clone())).flatten()
}

#[allow(dead_code)]
pub fn books_in_model(app: &AppWindow) -> usize {
    app.get_books().row_count()
}

#[allow(dead_code)]
pub fn data_dir() -> Option<PathBuf> {
    with_state(|st| st.cfg.data_dir.clone())
}
