//! Page request/result contract (spec §4) and the in-process renderer.

use std::path::{Path, PathBuf};

use cbz_reader::{CbzBook, CbzError};
use image_pipeline::{GrayImage, RenderError, RenderRequest, render_gray};
use memory_budget::{Limits, MemoryBudget};

#[derive(Debug, Clone, PartialEq)]
pub struct PageRequest {
    pub book_path: PathBuf,
    pub page_index: u32,
    pub render: RenderRequest,
    /// Incrementing id; results for older generations are discarded.
    pub generation: u64,
}

#[derive(Debug)]
pub struct RenderedPage {
    pub image: GrayImage,
    pub generation: u64,
    pub page_index: u32,
    pub decode_ms: u64,
    /// True if this came from the automatic low-memory retry.
    pub reduced_quality: bool,
}

#[derive(Debug)]
pub enum PageResult {
    Ready(RenderedPage),
    UnsupportedFormat(String),
    TooLargeForBudget(String),
    CorruptPage(String),
    Cancelled,
    InternalWorkerFailure(String),
    /// Book-level problem (missing file, invalid archive...).
    BookError(BookError),
}

impl PageResult {
    /// Worth one automatic retry at lower quality.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::TooLargeForBudget(_) | Self::InternalWorkerFailure(_)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BookErrorKind {
    NotFound,
    NoReadablePages,
    InvalidArchive,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookError {
    pub kind: BookErrorKind,
    pub message: String,
}

impl From<CbzError> for BookError {
    fn from(e: CbzError) -> Self {
        let kind = match &e {
            CbzError::Io(io) if io.kind() == std::io::ErrorKind::NotFound => {
                BookErrorKind::NotFound
            }
            CbzError::NoReadablePages => BookErrorKind::NoReadablePages,
            CbzError::InvalidArchive(_) | CbzError::TooManyEntries { .. } => {
                BookErrorKind::InvalidArchive
            }
            _ => BookErrorKind::Other,
        };
        Self {
            kind,
            message: e.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookInfo {
    pub page_count: u32,
    pub skipped_entries: usize,
}

/// Something that can open a book and render pages from it.
///
/// Implementations: [`InProcessRenderer`] (Milestone A, a worker thread) and
/// [`crate::protocol::ProcessRenderer`] (Milestone B, separate process).
pub trait PageRenderer: Send {
    fn open(&mut self, path: &Path) -> Result<BookInfo, BookError>;
    fn render(&mut self, req: &PageRequest, cancelled: &dyn Fn() -> bool) -> PageResult;
    /// Human-readable name for diagnostics.
    fn kind(&self) -> &'static str;
}

/// Decodes in the calling thread. Does **not** isolate decoder crashes or
/// OOM kills; that is what the process renderer is for.
pub struct InProcessRenderer {
    limits: Limits,
    budget: MemoryBudget,
    book: Option<CbzBook>,
}

impl InProcessRenderer {
    pub fn new(limits: Limits, budget: MemoryBudget) -> Self {
        Self {
            limits,
            budget,
            book: None,
        }
    }

    pub fn budget(&self) -> &MemoryBudget {
        &self.budget
    }

    fn ensure_open(&mut self, path: &Path) -> Result<&mut CbzBook, BookError> {
        if self.book.as_ref().and_then(|b| b.path()) != Some(path) {
            self.book = None; // drop the old archive before opening the next
            self.book = Some(CbzBook::open(path, self.limits)?);
        }
        Ok(self.book.as_mut().expect("just opened"))
    }
}

pub fn map_render_error(e: RenderError) -> PageResult {
    match e {
        RenderError::UnsupportedFormat(m) => PageResult::UnsupportedFormat(m),
        RenderError::TooLarge(m) => PageResult::TooLargeForBudget(m),
        RenderError::Corrupt(m) => PageResult::CorruptPage(m),
        RenderError::Cancelled => PageResult::Cancelled,
    }
}

impl PageRenderer for InProcessRenderer {
    fn open(&mut self, path: &Path) -> Result<BookInfo, BookError> {
        self.book = None;
        let book = self.ensure_open(path)?;
        Ok(BookInfo {
            page_count: book.page_count() as u32,
            skipped_entries: book.index().skipped.len(),
        })
    }

    fn render(&mut self, req: &PageRequest, cancelled: &dyn Fn() -> bool) -> PageResult {
        let started = std::time::Instant::now();
        let limits = self.limits;
        let budget = self.budget.clone();
        let book = match self.ensure_open(&req.book_path) {
            Ok(b) => b,
            Err(e) => return PageResult::BookError(e),
        };
        // Compressed input counts against the budget while it is alive.
        let entry_size = book
            .pages()
            .get(req.page_index as usize)
            .map(|p| p.uncompressed_size as usize)
            .unwrap_or(0);
        let input_reservation = match budget.reserve(entry_size, "compressed page") {
            Ok(r) => r,
            Err(e) => return PageResult::TooLargeForBudget(e.to_string()),
        };
        let bytes = match book.read_page(req.page_index as usize) {
            Ok((_, b)) => b,
            Err(CbzError::EntryTooLarge { .. } | CbzError::SuspiciousRatio { .. }) => {
                return PageResult::TooLargeForBudget("page exceeds archive safety limits".into());
            }
            Err(CbzError::CorruptEntry(m)) => return PageResult::CorruptPage(m),
            Err(e) => return PageResult::BookError(e.into()),
        };
        if cancelled() {
            return PageResult::Cancelled;
        }
        let result = render_gray(&bytes, &req.render, &limits, &budget, cancelled);
        drop(bytes);
        drop(input_reservation);
        match result {
            Ok(image) => PageResult::Ready(RenderedPage {
                image,
                generation: req.generation,
                page_index: req.page_index,
                decode_ms: started.elapsed().as_millis() as u64,
                reduced_quality: false,
            }),
            Err(e) => map_render_error(e),
        }
    }

    fn kind(&self) -> &'static str {
        "thread"
    }
}
