//! CBZ (ZIP of images) indexing and bounded page reads (spec §5, §24.1–24.2).
//!
//! * **Index** = read the ZIP central directory only. No entry is
//!   decompressed while indexing and no page is decoded.
//! * **Read** = decompress exactly one entry into a bounded buffer, after
//!   checking declared sizes, compression method, encryption and ratio.
//! * Entry names are **only** used as sort keys and for display. Nothing is
//!   ever extracted to the filesystem, so `../` names cannot escape.
//! * The archive file is opened read-only and is never modified.

use std::fmt;
use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::path::{Path, PathBuf};

use memory_budget::Limits;
use zip::CompressionMethod;
use zip::result::ZipError;

pub mod natural;

pub use natural::natural_cmp;

/// Image container detected from the entry name (magic bytes are checked
/// again by the decoder; the name is only a cheap pre-filter).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageKind {
    Jpeg,
    Png,
}

impl PageKind {
    fn from_name(name: &str) -> Option<Self> {
        let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
        match ext.as_str() {
            "jpg" | "jpeg" | "jpe" => Some(Self::Jpeg),
            "png" => Some(Self::Png),
            _ => None,
        }
    }
}

/// Why an entry was left out of the page list (diagnostics only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    Directory,
    NotAnImage,
    HiddenOrMetadata,
    Encrypted,
    UnsupportedCompression(String),
}

/// One readable page, referring to an entry of the central directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageEntry {
    pub archive_index: usize,
    pub name: String,
    pub kind: PageKind,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
}

#[derive(Debug)]
pub enum CbzError {
    /// File missing or unreadable. Saved progress must be kept.
    Io(std::io::Error),
    /// Not a ZIP, truncated central directory, etc.
    InvalidArchive(String),
    /// Structurally valid but contains nothing we can display.
    NoReadablePages,
    TooManyEntries {
        entries: usize,
        limit: usize,
    },
    PageOutOfRange {
        page: usize,
        page_count: usize,
    },
    /// Entry would exceed the configured uncompressed size limit.
    EntryTooLarge {
        declared: u64,
        limit: usize,
    },
    /// Suspicious compression ratio (possible decompression bomb).
    SuspiciousRatio {
        compressed: u64,
        uncompressed: u64,
    },
    /// Entry content did not match its central-directory record.
    CorruptEntry(String),
    Unsupported(String),
}

impl fmt::Display for CbzError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) if e.kind() == std::io::ErrorKind::NotFound => write!(f, "file not found"),
            Self::Io(e) => write!(f, "could not read file: {e}"),
            Self::InvalidArchive(m) => write!(f, "not a readable CBZ/ZIP archive: {m}"),
            Self::NoReadablePages => write!(f, "no readable pages"),
            Self::TooManyEntries { entries, limit } => {
                write!(f, "archive has {entries} entries (limit {limit})")
            }
            Self::PageOutOfRange { page, page_count } => {
                write!(f, "page {page} out of range (book has {page_count})")
            }
            Self::EntryTooLarge { declared, limit } => write!(
                f,
                "page is {} KiB uncompressed (limit {} KiB)",
                declared / 1024,
                limit / 1024
            ),
            Self::SuspiciousRatio {
                compressed,
                uncompressed,
            } => write!(
                f,
                "suspicious compression ratio ({uncompressed} bytes from {compressed})"
            ),
            Self::CorruptEntry(m) => write!(f, "corrupt page data: {m}"),
            Self::Unsupported(m) => write!(f, "unsupported: {m}"),
        }
    }
}

impl std::error::Error for CbzError {}

impl From<std::io::Error> for CbzError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

fn map_zip_error(e: ZipError) -> CbzError {
    match e {
        ZipError::Io(io) => CbzError::Io(io),
        ZipError::UnsupportedArchive(m) => CbzError::Unsupported(m.to_string()),
        other => CbzError::InvalidArchive(other.to_string()),
    }
}

fn is_hidden_or_metadata(name: &str) -> bool {
    name.split('/')
        .any(|part| part.starts_with('.') || part.eq_ignore_ascii_case("__MACOSX"))
        || name.rsplit('/').next().is_some_and(|f| {
            let f = f.to_ascii_lowercase();
            f.starts_with("thumb") || f == "cover-thumb.jpg"
        })
}

/// Result of indexing an archive.
#[derive(Debug, Clone)]
pub struct Index {
    pub pages: Vec<PageEntry>,
    pub skipped: Vec<(String, SkipReason)>,
    pub entry_count: usize,
}

/// Build the page index from central-directory metadata only.
pub fn index_archive<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    limits: &Limits,
) -> Result<Index, CbzError> {
    let entry_count = archive.len();
    if entry_count > limits.max_archive_entries {
        return Err(CbzError::TooManyEntries {
            entries: entry_count,
            limit: limits.max_archive_entries,
        });
    }
    let mut pages = Vec::new();
    let mut skipped = Vec::new();
    for i in 0..entry_count {
        // `by_index_raw` gives metadata without starting decompression.
        let entry = archive.by_index_raw(i).map_err(map_zip_error)?;
        let Ok(name) = entry.name().map(|n| n.into_owned()) else {
            skipped.push((
                format!("<entry {i}: undecodable name>"),
                SkipReason::NotAnImage,
            ));
            continue;
        };
        let reason = if entry.is_dir() {
            Some(SkipReason::Directory)
        } else if is_hidden_or_metadata(&name) {
            Some(SkipReason::HiddenOrMetadata)
        } else if PageKind::from_name(&name).is_none() {
            Some(SkipReason::NotAnImage)
        } else if entry.encrypted() {
            Some(SkipReason::Encrypted)
        } else if !matches!(
            entry.compression(),
            CompressionMethod::Stored | CompressionMethod::Deflated
        ) {
            Some(SkipReason::UnsupportedCompression(format!(
                "{:?}",
                entry.compression()
            )))
        } else {
            None
        };
        if let Some(reason) = reason {
            skipped.push((name, reason));
            continue;
        }
        if pages.len() >= limits.max_pages {
            return Err(CbzError::TooManyEntries {
                entries: entry_count,
                limit: limits.max_pages,
            });
        }
        pages.push(PageEntry {
            archive_index: i,
            kind: PageKind::from_name(&name).expect("filtered above"),
            compressed_size: entry.compressed_size(),
            uncompressed_size: entry.size(),
            name,
        });
    }
    pages.sort_by(|a, b| natural_cmp(&a.name, &b.name));
    if pages.is_empty() {
        return Err(CbzError::NoReadablePages);
    }
    Ok(Index {
        pages,
        skipped,
        entry_count,
    })
}

/// Check an entry against the limits *before* any decompression.
pub fn check_entry(entry: &PageEntry, limits: &Limits) -> Result<(), CbzError> {
    if entry.uncompressed_size > limits.uncompressed_entry_limit as u64 {
        return Err(CbzError::EntryTooLarge {
            declared: entry.uncompressed_size,
            limit: limits.uncompressed_entry_limit,
        });
    }
    if entry.uncompressed_size > limits.ratio_check_min_bytes as u64 {
        let ratio = entry.uncompressed_size / entry.compressed_size.max(1);
        if ratio > limits.max_compression_ratio {
            return Err(CbzError::SuspiciousRatio {
                compressed: entry.compressed_size,
                uncompressed: entry.uncompressed_size,
            });
        }
    }
    Ok(())
}

/// An opened CBZ: file handle + page index. Holds no decoded data.
pub struct CbzBook<R: Read + Seek = BufReader<File>> {
    path: Option<PathBuf>,
    archive: zip::ZipArchive<R>,
    index: Index,
    limits: Limits,
}

impl CbzBook<BufReader<File>> {
    /// Open read-only and index. Never writes to `path`.
    pub fn open(path: &Path, limits: Limits) -> Result<Self, CbzError> {
        let file = File::open(path)?;
        let mut book = Self::from_reader(BufReader::with_capacity(32 * 1024, file), limits)?;
        book.path = Some(path.to_owned());
        Ok(book)
    }
}

impl<R: Read + Seek> CbzBook<R> {
    pub fn from_reader(reader: R, limits: Limits) -> Result<Self, CbzError> {
        let mut archive = zip::ZipArchive::new(reader).map_err(map_zip_error)?;
        let index = index_archive(&mut archive, &limits)?;
        Ok(Self {
            path: None,
            archive,
            index,
            limits,
        })
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn page_count(&self) -> usize {
        self.index.pages.len()
    }

    pub fn pages(&self) -> &[PageEntry] {
        &self.index.pages
    }

    pub fn index(&self) -> &Index {
        &self.index
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Decompress exactly one page entry into memory, bounded by the limits.
    ///
    /// The declared size is checked first; the actual stream is then read
    /// through `take(limit + 1)` so a lying header cannot cause unbounded
    /// growth.
    pub fn read_page(&mut self, page: usize) -> Result<(PageEntry, Vec<u8>), CbzError> {
        let entry = self
            .index
            .pages
            .get(page)
            .cloned()
            .ok_or(CbzError::PageOutOfRange {
                page,
                page_count: self.page_count(),
            })?;
        check_entry(&entry, &self.limits)?;
        let limit = self.limits.uncompressed_entry_limit;
        let file = self
            .archive
            .by_index(entry.archive_index)
            .map_err(map_zip_error)?;
        let mut buf = Vec::with_capacity(entry.uncompressed_size as usize);
        let read = file
            .take(limit as u64 + 1)
            .read_to_end(&mut buf)
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::InvalidData
                | std::io::ErrorKind::UnexpectedEof
                | std::io::ErrorKind::Other => CbzError::CorruptEntry(e.to_string()),
                _ => CbzError::Io(e),
            })?;
        if read > limit {
            return Err(CbzError::EntryTooLarge {
                declared: read as u64,
                limit,
            });
        }
        if read as u64 != entry.uncompressed_size {
            return Err(CbzError::CorruptEntry(format!(
                "expected {} bytes, got {read}",
                entry.uncompressed_size
            )));
        }
        Ok((entry, buf))
    }
}

#[cfg(test)]
mod tests;
