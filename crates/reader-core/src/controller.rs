//! Event-driven reader controller. Owns progress state and navigation and
//! translates UI input + scheduler events into [`Output`]s that the UI layer
//! executes. Contains no UI-toolkit code, so it is fully unit-testable.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

use image_pipeline::{GrayImage, Quality, RenderRequest};
use persistence::{ContrastPref, Direction, Fit, ReadingPrefs, Store, unix_now};

use crate::catalog::CatalogEntry;
use crate::navigation::{
    NavAction, Navigator, Step, TapGate, action_for, segment_offsets, zone_for,
};
use crate::render::{BookErrorKind, PageRequest, PageResult};
use crate::scheduler::{Event, Job};
use crate::state::{ReaderEvent, ReaderState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorView {
    pub message: String,
    pub can_skip: bool,
    pub can_retry: bool,
}

#[derive(Debug)]
pub enum Output {
    Submit(Job),
    /// Drop queued/in-flight work (`Scheduler::cancel_all`).
    CancelAll(u64),
    /// Reader opened; image not ready yet.
    EnterReader {
        title: String,
    },
    /// A page decode started (the UI may show a delayed, unobtrusive hint).
    Loading {
        page: u32,
    },
    /// New page image. `offset_px` is the vertical scroll in display pixels
    /// (fit width), `display_height` the scaled image height.
    Present {
        image: GrayImage,
        page: u32,
        page_count: u32,
        offset_px: f32,
        display_height: f32,
        reduced_quality: bool,
        decode_ms: u64,
    },
    /// Same image, different segment.
    Scroll {
        offset_px: f32,
    },
    ShowError(ErrorView),
    ShowOptions(bool),
    Prefs(ReadingPrefs),
    /// Brief status text, e.g. "Last page".
    Notice(String),
    /// State changed; schedule a debounced flush.
    Persist,
    ToLibrary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryItem {
    pub id: String,
    pub title: String,
    /// 1-based for display; 0 = never opened.
    pub page: u32,
    pub page_count: u32,
    pub missing: bool,
}

impl LibraryItem {
    pub fn progress_text(&self) -> String {
        if self.missing {
            "File not found — progress kept".into()
        } else if self.page_count == 0 || self.page == 0 {
            "Not started".into()
        } else {
            format!("Page {} / {}", self.page, self.page_count)
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryModel {
    pub continue_reading: Option<LibraryItem>,
    pub books: Vec<LibraryItem>,
}

struct Session {
    id: String,
    path: PathBuf,
    nav: Navigator,
    prefs: ReadingPrefs,
    offsets: Vec<f32>,
    opened: bool,
}

pub struct Controller {
    store: Store,
    viewport: Viewport,
    generation: u64,
    state: ReaderState,
    session: Option<Session>,
    gate: TapGate,
    present_paths: HashSet<PathBuf>,
}

pub const MSG_TOO_LARGE: &str =
    "This page needs too much memory. Try a lower quality preview or skip this page.";
pub const MSG_CORRUPT: &str = "This page is damaged and can't be shown.";
pub const MSG_UNSUPPORTED: &str = "This page uses an image format the reader doesn't support.";
pub const MSG_WORKER: &str = "The page renderer stopped unexpectedly.";
pub const MSG_NOT_FOUND: &str =
    "This book's file can't be found. Your reading progress has been kept.";
pub const MSG_NO_PAGES: &str = "This book has no readable pages.";
pub const MSG_INVALID: &str = "This file isn't a readable CBZ archive.";

impl Controller {
    pub fn new(store: Store, viewport: Viewport) -> Self {
        Self {
            store,
            viewport,
            generation: 0,
            state: ReaderState::Idle,
            session: None,
            gate: TapGate::default(),
            present_paths: HashSet::new(),
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }
    pub fn store_mut(&mut self) -> &mut Store {
        &mut self.store
    }
    pub fn state(&self) -> ReaderState {
        self.state
    }
    pub fn viewport(&self) -> Viewport {
        self.viewport
    }
    /// Takes effect for the next page request (window size is only known
    /// once the platform has started).
    pub fn set_viewport(&mut self, v: Viewport) {
        if v.width > 0 && v.height > 0 {
            self.viewport = v;
        }
    }
    pub fn prefs(&self) -> Option<ReadingPrefs> {
        self.session.as_ref().map(|s| s.prefs)
    }
    pub fn position(&self) -> Option<(u32, u32)> {
        self.session
            .as_ref()
            .filter(|s| s.opened)
            .map(|s| (s.nav.page(), s.nav.page_count()))
    }

    fn transition(&mut self, ev: ReaderEvent) {
        if let Some(s) = self.state.on(ev) {
            self.state = s;
        }
    }

    fn next_generation(&mut self) -> u64 {
        self.generation += 1;
        self.generation
    }

    // ---------------------------------------------------------------- library

    /// Merge a fresh folder scan into the store (keeps progress of missing
    /// books; relinks progress when a file changed in place or was renamed).
    pub fn sync_catalog(&mut self, catalog: &[CatalogEntry]) {
        self.present_paths = catalog.iter().map(|e| e.path.clone()).collect();
        for e in catalog {
            if self.store.book(&e.id).is_some() {
                self.store
                    .upsert_book(&e.id, &e.path, &e.title, e.size, e.mtime);
                continue;
            }
            self.store
                .upsert_book(&e.id, &e.path, &e.title, e.size, e.mtime);
            // Same path, different id → file was modified; carry progress over.
            // Same size + title at a vanished path → file was moved/renamed.
            let old = self
                .store
                .state()
                .books
                .values()
                .find(|b| {
                    b.id != e.id
                        && b.last_opened_at > 0
                        && (b.path == e.path
                            || (!self.present_paths.contains(&b.path)
                                && b.size == e.size
                                && b.title == e.title))
                })
                .map(|b| b.id.clone());
            if let Some(old) = old {
                self.store.relink(&old, &e.id);
            }
        }
    }

    pub fn library(&self) -> LibraryModel {
        let item = |b: &persistence::BookRecord| LibraryItem {
            id: b.id.clone(),
            title: b.title.clone(),
            page: if b.last_opened_at > 0 {
                b.last_read_page + 1
            } else {
                0
            },
            page_count: b.page_count,
            missing: !self.present_paths.contains(&b.path),
        };
        let state = self.store.state();
        let mut books: Vec<LibraryItem> = state
            .books
            .values()
            .filter(|b| self.present_paths.contains(&b.path) || b.last_opened_at > 0)
            .map(item)
            .collect();
        books.sort_by(|a, b| cbz_reader::natural_cmp(&a.title, &b.title));
        LibraryModel {
            continue_reading: state.last_opened().map(item),
            books,
        }
    }

    // ----------------------------------------------------------------- reader

    pub fn open(&mut self, id: &str) -> Vec<Output> {
        let Some(rec) = self.store.book(id).cloned() else {
            return vec![Output::Notice("Book not found in library".into())];
        };
        let generation = self.next_generation();
        self.state = ReaderState::Idle;
        self.transition(ReaderEvent::Open);
        self.session = Some(Session {
            id: rec.id.clone(),
            path: rec.path.clone(),
            nav: Navigator::new(rec.page_count.max(1), rec.last_read_page),
            prefs: rec.prefs,
            offsets: vec![0.0],
            opened: false,
        });
        vec![
            Output::EnterReader { title: rec.title },
            Output::Prefs(rec.prefs),
            Output::Submit(Job::Open {
                path: rec.path,
                generation,
            }),
        ]
    }

    fn request_page(&mut self, quality: Quality) -> Vec<Output> {
        let generation = self.next_generation();
        let vp = self.viewport;
        let Some(s) = self.session.as_ref() else {
            return vec![];
        };
        let req = PageRequest {
            book_path: s.path.clone(),
            page_index: s.nav.page(),
            render: RenderRequest {
                viewport_width: vp.width,
                viewport_height: vp.height,
                fit: crate::fit_mode(s.prefs.fit),
                quality,
                contrast: crate::contrast(s.prefs.contrast),
            },
            generation,
        };
        let page = s.nav.page();
        self.transition(ReaderEvent::RequestPage);
        vec![Output::Loading { page }, Output::Submit(Job::Render(req))]
    }

    fn error(&mut self, message: &str, can_skip: bool, can_retry: bool) -> Vec<Output> {
        self.transition(ReaderEvent::Fail);
        vec![Output::ShowError(ErrorView {
            message: message.into(),
            can_skip,
            can_retry,
        })]
    }

    pub fn on_event(&mut self, ev: Event) -> Vec<Output> {
        match ev {
            Event::Opened {
                generation, result, ..
            } if generation == self.generation => match result {
                Ok(info) => {
                    let Some(s) = self.session.as_mut() else {
                        return vec![];
                    };
                    let saved = self
                        .store
                        .book(&s.id)
                        .map(|b| b.last_read_page)
                        .unwrap_or(0);
                    s.nav = Navigator::new(info.page_count, saved);
                    s.opened = true;
                    let id = s.id.clone();
                    self.store.update(|st| {
                        if let Some(b) = st.books.get_mut(&id) {
                            b.page_count = info.page_count;
                            b.last_opened_at = unix_now();
                        }
                    });
                    self.transition(ReaderEvent::Indexed);
                    let mut out = vec![Output::Persist];
                    out.extend(self.request_page(Quality::Normal));
                    out
                }
                Err(e) => {
                    let msg = match e.kind {
                        BookErrorKind::NotFound => MSG_NOT_FOUND,
                        BookErrorKind::NoReadablePages => MSG_NO_PAGES,
                        BookErrorKind::InvalidArchive => MSG_INVALID,
                        BookErrorKind::Other => MSG_INVALID,
                    };
                    self.error(msg, false, true)
                }
            },
            Event::Page {
                generation,
                page_index,
                result,
                elapsed_ms,
            } if generation == self.generation => self.on_page(page_index, result, elapsed_ms),
            // Stale: belongs to a superseded request.
            _ => vec![],
        }
    }

    fn on_page(&mut self, page_index: u32, result: PageResult, elapsed_ms: u64) -> Vec<Output> {
        let vp = self.viewport;
        let Some(s) = self.session.as_mut() else {
            return vec![];
        };
        if page_index != s.nav.page() {
            return vec![];
        }
        match result {
            PageResult::Ready(p) => {
                let (w, h) = (p.image.width.max(1) as f32, p.image.height as f32);
                let display_height = match s.prefs.fit {
                    Fit::Width => h * vp.width as f32 / w,
                    Fit::Page => h.min(vp.height as f32),
                };
                s.offsets = match s.prefs.fit {
                    Fit::Width => segment_offsets(display_height, vp.height as f32),
                    Fit::Page => vec![0.0],
                };
                s.nav.set_segments(s.offsets.len() as u32);
                let offset_px = s.offsets[s.nav.segment() as usize];
                let (page, page_count, id) = (s.nav.page(), s.nav.page_count(), s.id.clone());
                self.store.update(|st| {
                    if let Some(b) = st.books.get_mut(&id) {
                        b.last_read_page = page;
                    }
                });
                self.transition(ReaderEvent::Presented);
                vec![
                    Output::Present {
                        image: p.image,
                        page,
                        page_count,
                        offset_px,
                        display_height,
                        reduced_quality: p.reduced_quality,
                        decode_ms: elapsed_ms.max(p.decode_ms),
                    },
                    Output::Persist,
                ]
            }
            PageResult::TooLargeForBudget(_) => self.error(MSG_TOO_LARGE, true, true),
            PageResult::CorruptPage(_) => self.error(MSG_CORRUPT, true, false),
            PageResult::UnsupportedFormat(_) => self.error(MSG_UNSUPPORTED, true, false),
            PageResult::InternalWorkerFailure(_) => self.error(MSG_WORKER, true, true),
            PageResult::BookError(e) if e.kind == BookErrorKind::NotFound => {
                self.error(MSG_NOT_FOUND, false, false)
            }
            PageResult::BookError(_) => self.error(MSG_INVALID, false, false),
            PageResult::Cancelled => vec![],
        }
    }

    fn step(&mut self, step: Step) -> Vec<Output> {
        match step {
            Step::Segment(i) => {
                let off = self
                    .session
                    .as_ref()
                    .and_then(|s| s.offsets.get(i as usize).copied())
                    .unwrap_or(0.0);
                vec![Output::Scroll { offset_px: off }]
            }
            Step::Page(_) => self.request_page(Quality::Normal),
            Step::AtEnd => vec![Output::Notice("Last page".into())],
            Step::AtStart => vec![Output::Notice("First page".into())],
        }
    }

    fn reading(&self) -> bool {
        self.session.as_ref().is_some_and(|s| s.opened)
    }

    pub fn next_page(&mut self) -> Vec<Output> {
        if !self.reading() {
            return vec![];
        }
        let step = self.session.as_mut().expect("reading").nav.forward();
        self.step(step)
    }

    pub fn previous_page(&mut self) -> Vec<Output> {
        if !self.reading() {
            return vec![];
        }
        let step = self.session.as_mut().expect("reading").nav.backward();
        self.step(step)
    }

    /// Tap on the page at `x` (display pixels). Duplicate taps during a
    /// decode or within the debounce interval are ignored.
    pub fn tap(&mut self, x: f32, now: Instant) -> Vec<Output> {
        let Some(dir) = self.session.as_ref().map(|s| s.prefs.direction) else {
            return vec![];
        };
        match action_for(zone_for(x, self.viewport.width as f32), dir) {
            NavAction::ShowOptions => vec![Output::ShowOptions(true)],
            action => {
                if !self.gate.accept(now, self.state.is_busy()) {
                    return vec![];
                }
                match action {
                    NavAction::Next => self.next_page(),
                    NavAction::Previous => self.previous_page(),
                    NavAction::ShowOptions => unreachable!(),
                }
            }
        }
    }

    pub fn jump_to(&mut self, page: u32) -> Vec<Output> {
        if !self.reading() {
            return vec![];
        }
        let step = self.session.as_mut().expect("reading").nav.jump(page);
        self.step(step)
    }

    /// Coarse progress jump, `fraction` in 0..=1.
    pub fn jump_fraction(&mut self, fraction: f32) -> Vec<Output> {
        let Some((_, count)) = self.position() else {
            return vec![];
        };
        let page = (fraction.clamp(0.0, 1.0) * (count.saturating_sub(1)) as f32).round() as u32;
        self.jump_to(page)
    }

    /// Skip a page that failed to render.
    pub fn skip_page(&mut self) -> Vec<Output> {
        let Some((page, count)) = self.position() else {
            return vec![];
        };
        if page + 1 >= count {
            return vec![Output::Notice("Last page".into())];
        }
        self.jump_to(page + 1)
    }

    /// Re-request the current page (or reopen the book) at lower quality.
    pub fn retry(&mut self) -> Vec<Output> {
        if self.reading() {
            return self.request_page(Quality::LowMemory);
        }
        match self.session.as_ref().map(|s| s.id.clone()) {
            Some(id) => self.open(&id),
            None => vec![],
        }
    }

    fn update_prefs(&mut self, f: impl FnOnce(&mut ReadingPrefs), rerender: bool) -> Vec<Output> {
        let Some(s) = self.session.as_mut() else {
            return vec![];
        };
        f(&mut s.prefs);
        let (id, prefs) = (s.id.clone(), s.prefs);
        self.store.update(|st| {
            if let Some(b) = st.books.get_mut(&id) {
                b.prefs = prefs;
            }
        });
        let mut out = vec![Output::Prefs(prefs), Output::Persist];
        if rerender && self.reading() {
            out.extend(self.request_page(Quality::Normal));
        }
        out
    }

    pub fn set_direction(&mut self, d: Direction) -> Vec<Output> {
        self.update_prefs(|p| p.direction = d, false)
    }

    pub fn set_fit(&mut self, fit: Fit) -> Vec<Output> {
        if self.prefs().is_some_and(|p| p.fit == fit) {
            return vec![];
        }
        self.update_prefs(|p| p.fit = fit, true)
    }

    pub fn set_contrast(&mut self, c: ContrastPref) -> Vec<Output> {
        if self.prefs().is_some_and(|p| p.contrast == c) {
            return vec![];
        }
        self.update_prefs(|p| p.contrast = c, true)
    }

    pub fn set_show_progress(&mut self, on: bool) -> Vec<Output> {
        self.update_prefs(|p| p.show_progress = on, false)
    }

    /// Leave the reader. Cancels in-flight work so its result is dropped.
    pub fn close(&mut self) -> Vec<Output> {
        let generation = self.next_generation();
        self.transition(ReaderEvent::Close);
        self.transition(ReaderEvent::Closed);
        self.session = None;
        vec![
            Output::CancelAll(generation),
            Output::Persist,
            Output::ToLibrary,
        ]
    }
}

#[cfg(test)]
mod tests;
