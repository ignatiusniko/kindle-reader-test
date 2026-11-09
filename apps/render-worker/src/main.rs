//! `render-worker`: decodes pages in a separate process so a decoder crash,
//! allocation failure or OOM kill does not take the UI down (spec §4, §24.7).
//!
//! Protocol: see `reader_core::protocol`. Options:
//!
//! * `--budget BYTES`               reservation budget for decode scratch
//! * `--address-space-limit BYTES`  hard `RLIMIT_AS` for this process; an
//!   allocation beyond it aborts **only the worker**
//!
//! Fault injection for isolation tests (debug builds only):
//! `--debug-abort-on-page N` aborts when asked to render page `N`.

// musl's allocator fragments under repeated page decodes (RSS grew without
// bound in MEM-01); mimalloc keeps it flat. See docs/decisions/0003.
#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::io::{BufReader, BufWriter};

use memory_budget::{Limits, MemoryBudget};
use reader_core::render::InProcessRenderer;

fn parse_bytes(v: Option<String>, name: &str) -> Option<usize> {
    let v = v?;
    match v.parse() {
        Ok(n) => Some(n),
        Err(_) => {
            eprintln!("render-worker: invalid {name}: {v}");
            std::process::exit(2);
        }
    }
}

fn set_address_space_limit(bytes: usize) {
    let lim = libc::rlimit {
        rlim_cur: bytes as libc::rlim_t,
        rlim_max: bytes as libc::rlim_t,
    };
    // SAFETY: plain syscall with a valid pointer to a stack value.
    let rc = unsafe { libc::setrlimit(libc::RLIMIT_AS, &lim) };
    if rc != 0 {
        eprintln!(
            "render-worker: setrlimit(RLIMIT_AS) failed: {}",
            std::io::Error::last_os_error()
        );
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut budget = None;
    let mut as_limit = None;
    let mut abort_page: Option<u32> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--budget" => budget = parse_bytes(args.next(), "--budget"),
            "--address-space-limit" => as_limit = parse_bytes(args.next(), "--address-space-limit"),
            "--debug-abort-on-page" if cfg!(debug_assertions) => {
                abort_page = args.next().and_then(|v| v.parse().ok());
            }
            "--version" => {
                println!("render-worker {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            other => {
                eprintln!("render-worker: unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    if let Some(limit) = as_limit {
        set_address_space_limit(limit);
    }
    let budget = budget.unwrap_or(memory_budget::BudgetPlan::conservative_default().decode_budget);
    let mut renderer = InProcessRenderer::new(Limits::default(), MemoryBudget::new(budget));
    let stdin = BufReader::new(std::io::stdin().lock());
    let stdout = BufWriter::with_capacity(64 * 1024, std::io::stdout().lock());
    let result = if let Some(page) = abort_page {
        reader_core::protocol::serve(FaultInjector { inner: stdin, page }, stdout, &mut renderer)
    } else {
        reader_core::protocol::serve(stdin, stdout, &mut renderer)
    };
    if let Err(e) = result
        && e.kind() != std::io::ErrorKind::BrokenPipe
    {
        eprintln!("render-worker: {e}");
        std::process::exit(1);
    }
}

/// Wraps stdin and aborts the process when a render request for `page`
/// arrives. Only reachable via the debug-only flag (tests).
struct FaultInjector<R> {
    inner: R,
    page: u32,
}

impl<R: std::io::Read> std::io::Read for FaultInjector<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

impl<R: std::io::BufRead> std::io::BufRead for FaultInjector<R> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        let page = self.page;
        let buf = self.inner.fill_buf()?;
        let needle = format!("\"page\":{page},");
        if buf.windows(needle.len()).any(|w| w == needle.as_bytes()) {
            std::process::abort();
        }
        Ok(buf)
    }
    fn consume(&mut self, amt: usize) {
        self.inner.consume(amt)
    }
}
