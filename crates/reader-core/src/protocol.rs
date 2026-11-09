//! Worker-process IPC (Milestone B, spec §4 "Concurrency and crash
//! isolation", §24.7).
//!
//! Transport: the worker's stdin/stdout pipes. Each request is one JSON line.
//! Each response is one JSON line; a `ready` response is followed by exactly
//! `width × height` raw Gray8 bytes. The UI side validates the announced size
//! against the request's viewport before reading, so the payload is bounded
//! and copied once.
//!
//! A worker crash (decoder bug, `RLIMIT_AS` hit, OOM kill) shows up as EOF
//! or a broken pipe and becomes [`PageResult::InternalWorkerFailure`]; the
//! scheduler then retries once at lower quality, and the renderer stops
//! respawning after repeated consecutive failures (no restart loop).

use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use image_pipeline::{GrayImage, Quality, RenderRequest};
use memory_budget::{Limits, MemoryBudget, required_bytes};
use persistence::{ContrastPref, Fit};
use serde::{Deserialize, Serialize};

use crate::render::{
    BookError, BookErrorKind, BookInfo, InProcessRenderer, PageRenderer, PageRequest, PageResult,
    RenderedPage,
};

const MAX_HEADER_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum WorkerRequest {
    Ping,
    Open {
        path: PathBuf,
    },
    Render {
        path: PathBuf,
        page: u32,
        viewport_width: u32,
        viewport_height: u32,
        fit: Fit,
        contrast: ContrastPref,
        low_memory: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireError {
    Unsupported,
    TooLarge,
    Corrupt,
    BookNotFound,
    BookNoPages,
    BookInvalid,
    BookOther,
    Protocol,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum WorkerResponse {
    Pong {
        pid: u32,
    },
    Opened {
        page_count: u32,
        skipped_entries: usize,
    },
    /// Followed by `width * height` bytes of Gray8 pixels.
    Ready {
        width: u32,
        height: u32,
        decode_ms: u64,
    },
    Error {
        kind: WireError,
        message: String,
    },
}

fn book_error_to_wire(e: &BookError) -> WireError {
    match e.kind {
        BookErrorKind::NotFound => WireError::BookNotFound,
        BookErrorKind::NoReadablePages => WireError::BookNoPages,
        BookErrorKind::InvalidArchive => WireError::BookInvalid,
        BookErrorKind::Other => WireError::BookOther,
    }
}

fn wire_to_book_error(kind: WireError, message: String) -> Option<BookError> {
    let kind = match kind {
        WireError::BookNotFound => BookErrorKind::NotFound,
        WireError::BookNoPages => BookErrorKind::NoReadablePages,
        WireError::BookInvalid => BookErrorKind::InvalidArchive,
        WireError::BookOther => BookErrorKind::Other,
        _ => return None,
    };
    Some(BookError { kind, message })
}

fn to_render_request(fit: Fit, contrast: ContrastPref, low: bool, w: u32, h: u32) -> RenderRequest {
    RenderRequest {
        viewport_width: w,
        viewport_height: h,
        fit: crate::fit_mode(fit),
        quality: if low {
            Quality::LowMemory
        } else {
            Quality::Normal
        },
        contrast: crate::contrast(contrast),
    }
}

fn write_json<W: Write>(out: &mut W, v: &WorkerResponse) -> io::Result<()> {
    serde_json::to_writer(&mut *out, v).map_err(io::Error::other)?;
    out.write_all(b"\n")
}

/// Worker-side loop. Returns when stdin closes.
pub fn serve<R: BufRead, W: Write>(
    mut input: R,
    mut output: W,
    renderer: &mut InProcessRenderer,
) -> io::Result<()> {
    let mut line = String::new();
    loop {
        line.clear();
        let n = (&mut input).take(MAX_HEADER_BYTES).read_line(&mut line)?;
        if n == 0 {
            return Ok(());
        }
        let req: WorkerRequest = match serde_json::from_str(line.trim_end()) {
            Ok(r) => r,
            Err(e) => {
                write_json(
                    &mut output,
                    &WorkerResponse::Error {
                        kind: WireError::Protocol,
                        message: e.to_string(),
                    },
                )?;
                output.flush()?;
                continue;
            }
        };
        match req {
            WorkerRequest::Ping => write_json(
                &mut output,
                &WorkerResponse::Pong {
                    pid: std::process::id(),
                },
            )?,
            WorkerRequest::Open { path } => {
                let resp = match renderer.open(&path) {
                    Ok(info) => WorkerResponse::Opened {
                        page_count: info.page_count,
                        skipped_entries: info.skipped_entries,
                    },
                    Err(e) => WorkerResponse::Error {
                        kind: book_error_to_wire(&e),
                        message: e.message,
                    },
                };
                write_json(&mut output, &resp)?;
            }
            WorkerRequest::Render {
                path,
                page,
                viewport_width,
                viewport_height,
                fit,
                contrast,
                low_memory,
            } => {
                let req = PageRequest {
                    book_path: path,
                    page_index: page,
                    render: to_render_request(
                        fit,
                        contrast,
                        low_memory,
                        viewport_width,
                        viewport_height,
                    ),
                    generation: 0,
                };
                let resp = renderer.render(&req, &|| false);
                let (kind, message) = match resp {
                    PageResult::Ready(p) => {
                        write_json(
                            &mut output,
                            &WorkerResponse::Ready {
                                width: p.image.width,
                                height: p.image.height,
                                decode_ms: p.decode_ms,
                            },
                        )?;
                        output.write_all(&p.image.pixels)?;
                        output.flush()?;
                        continue;
                    }
                    PageResult::UnsupportedFormat(m) => (WireError::Unsupported, m),
                    PageResult::TooLargeForBudget(m) => (WireError::TooLarge, m),
                    PageResult::CorruptPage(m) => (WireError::Corrupt, m),
                    PageResult::BookError(e) => (book_error_to_wire(&e), e.message),
                    PageResult::Cancelled | PageResult::InternalWorkerFailure(_) => {
                        (WireError::Protocol, "unexpected worker state".into())
                    }
                };
                write_json(&mut output, &WorkerResponse::Error { kind, message })?;
            }
        }
        output.flush()?;
    }
}

struct WorkerChild {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Drop for WorkerChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// UI-side handle to a `render-worker` process.
pub struct ProcessRenderer {
    exe: PathBuf,
    args: Vec<String>,
    child: Option<WorkerChild>,
    consecutive_failures: u32,
    max_consecutive_failures: u32,
    limits: Limits,
    /// UI-side budget for received page buffers.
    budget: MemoryBudget,
}

impl ProcessRenderer {
    pub fn new(exe: PathBuf, args: Vec<String>, limits: Limits, budget: MemoryBudget) -> Self {
        Self {
            exe,
            args,
            child: None,
            consecutive_failures: 0,
            max_consecutive_failures: 3,
            limits,
            budget,
        }
    }

    /// PID of the running worker, if any (diagnostics / kill tests).
    pub fn worker_pid(&self) -> Option<u32> {
        self.child.as_ref().map(|c| c.child.id())
    }

    fn ensure_child(&mut self) -> io::Result<&mut WorkerChild> {
        if self.child.is_none() {
            let mut child = Command::new(&self.exe)
                .args(&self.args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()?;
            let stdin = child.stdin.take().expect("piped");
            let stdout = BufReader::new(child.stdout.take().expect("piped"));
            self.child = Some(WorkerChild {
                child,
                stdin,
                stdout,
            });
        }
        Ok(self.child.as_mut().expect("just spawned"))
    }

    fn call(&mut self, req: &WorkerRequest) -> io::Result<WorkerResponse> {
        let c = self.ensure_child()?;
        serde_json::to_writer(&mut c.stdin, req).map_err(io::Error::other)?;
        c.stdin.write_all(b"\n")?;
        c.stdin.flush()?;
        let mut line = String::new();
        let n = (&mut c.stdout)
            .take(MAX_HEADER_BYTES)
            .read_line(&mut line)?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "worker exited",
            ));
        }
        serde_json::from_str(line.trim_end())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    fn fail(&mut self, e: io::Error) -> String {
        self.child = None; // kills + reaps
        self.consecutive_failures += 1;
        format!("render worker failed: {e}")
    }

    fn read_pixels(&mut self, w: u32, h: u32) -> io::Result<Vec<u8>> {
        let n = required_bytes(w, h, 1)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "size overflow"))?;
        let c = self.child.as_mut().expect("child alive after call");
        let mut buf = vec![0u8; n];
        c.stdout.read_exact(&mut buf)?;
        Ok(buf)
    }

    /// Ask the worker for its PID (spawning it if necessary).
    pub fn ping(&mut self) -> io::Result<u32> {
        match self.call(&WorkerRequest::Ping)? {
            WorkerResponse::Pong { pid } => Ok(pid),
            other => Err(io::Error::other(format!("unexpected {other:?}"))),
        }
    }
}

impl PageRenderer for ProcessRenderer {
    fn open(&mut self, path: &Path) -> Result<BookInfo, BookError> {
        self.consecutive_failures = 0;
        match self.call(&WorkerRequest::Open {
            path: path.to_owned(),
        }) {
            Ok(WorkerResponse::Opened {
                page_count,
                skipped_entries,
            }) => Ok(BookInfo {
                page_count,
                skipped_entries,
            }),
            Ok(WorkerResponse::Error { kind, message }) => {
                Err(
                    wire_to_book_error(kind, message.clone()).unwrap_or(BookError {
                        kind: BookErrorKind::Other,
                        message,
                    }),
                )
            }
            Ok(other) => Err(BookError {
                kind: BookErrorKind::Other,
                message: format!("unexpected worker reply {other:?}"),
            }),
            Err(e) => Err(BookError {
                kind: BookErrorKind::Other,
                message: self.fail(e),
            }),
        }
    }

    fn render(&mut self, req: &PageRequest, cancelled: &dyn Fn() -> bool) -> PageResult {
        if self.consecutive_failures >= self.max_consecutive_failures {
            return PageResult::InternalWorkerFailure(
                "renderer stopped after repeated failures; reopen the book to try again".into(),
            );
        }
        if cancelled() {
            return PageResult::Cancelled;
        }
        let wire = WorkerRequest::Render {
            path: req.book_path.clone(),
            page: req.page_index,
            viewport_width: req.render.viewport_width,
            viewport_height: req.render.viewport_height,
            fit: match req.render.fit {
                image_pipeline::FitMode::Page => Fit::Page,
                image_pipeline::FitMode::Width => Fit::Width,
            },
            contrast: match req.render.contrast {
                image_pipeline::Contrast::Normal => ContrastPref::Normal,
                image_pipeline::Contrast::Darker => ContrastPref::Darker,
            },
            low_memory: req.render.quality == Quality::LowMemory,
        };
        match self.call(&wire) {
            Ok(WorkerResponse::Ready {
                width,
                height,
                decode_ms,
            }) => {
                let pixels = width as u64 * height as u64;
                if width == 0 || height == 0 || pixels > self.limits.max_output_pixels {
                    let e = io::Error::new(io::ErrorKind::InvalidData, "oversized reply");
                    return PageResult::InternalWorkerFailure(self.fail(e));
                }
                let reservation = match self.budget.reserve(pixels as usize, "IPC page buffer") {
                    Ok(r) => r,
                    Err(e) => {
                        // Must still drain the payload; simplest is to restart the worker.
                        self.child = None;
                        return PageResult::TooLargeForBudget(e.to_string());
                    }
                };
                match self.read_pixels(width, height) {
                    Ok(px) => {
                        self.consecutive_failures = 0;
                        let image = GrayImage::from_pixels(width, height, px)
                            .expect("length checked")
                            .with_reservation(reservation);
                        PageResult::Ready(RenderedPage {
                            image,
                            generation: req.generation,
                            page_index: req.page_index,
                            decode_ms,
                            reduced_quality: false,
                        })
                    }
                    Err(e) => PageResult::InternalWorkerFailure(self.fail(e)),
                }
            }
            Ok(WorkerResponse::Error { kind, message }) => {
                self.consecutive_failures = 0;
                match kind {
                    WireError::Unsupported => PageResult::UnsupportedFormat(message),
                    WireError::TooLarge => PageResult::TooLargeForBudget(message),
                    WireError::Corrupt => PageResult::CorruptPage(message),
                    WireError::Protocol => PageResult::InternalWorkerFailure(message),
                    k => PageResult::BookError(wire_to_book_error(k, message).expect("book kind")),
                }
            }
            Ok(other) => {
                let e = io::Error::other(format!("unexpected worker reply {other:?}"));
                PageResult::InternalWorkerFailure(self.fail(e))
            }
            Err(e) => PageResult::InternalWorkerFailure(self.fail(e)),
        }
    }

    fn kind(&self) -> &'static str {
        "process"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_format_is_stable() {
        let r = WorkerRequest::Render {
            path: "/b.cbz".into(),
            page: 3,
            viewport_width: 1072,
            viewport_height: 1448,
            fit: Fit::Width,
            contrast: ContrastPref::Normal,
            low_memory: true,
        };
        let s = serde_json::to_string(&r).unwrap();
        assert_eq!(
            s,
            r#"{"op":"render","path":"/b.cbz","page":3,"viewport_width":1072,"viewport_height":1448,"fit":"width","contrast":"normal","low_memory":true}"#
        );
        let resp = WorkerResponse::Ready {
            width: 2,
            height: 1,
            decode_ms: 5,
        };
        assert_eq!(
            serde_json::to_string(&resp).unwrap(),
            r#"{"status":"ready","width":2,"height":1,"decode_ms":5}"#
        );
    }

    #[test]
    fn serve_handles_bad_lines_and_missing_books() {
        let input =
            b"not json\n{\"op\":\"open\",\"path\":\"/nonexistent.cbz\"}\n{\"op\":\"ping\"}\n";
        let mut out = Vec::new();
        let mut r = InProcessRenderer::new(Limits::default(), MemoryBudget::new(1 << 20));
        serve(&input[..], &mut out, &mut r).unwrap();
        let lines: Vec<WorkerResponse> = out
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
            .map(|l| serde_json::from_slice(l).unwrap())
            .collect();
        assert!(matches!(
            lines[0],
            WorkerResponse::Error {
                kind: WireError::Protocol,
                ..
            }
        ));
        assert!(matches!(
            lines[1],
            WorkerResponse::Error {
                kind: WireError::BookNotFound,
                ..
            }
        ));
        assert!(matches!(lines[2], WorkerResponse::Pong { .. }));
    }
}
