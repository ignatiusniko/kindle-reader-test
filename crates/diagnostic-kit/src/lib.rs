//! Local, opt-in diagnostics (spec §4 `diagnostics`, §26.5–26.6).
//!
//! Nothing here sends data anywhere. Logs are size-bounded and should not
//! contain book paths unless the user opts in.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub use memory_budget::meminfo::MemInfo;

/// Fields from `/proc/<pid>/status`, in KiB. `None` = not reported.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProcStatus {
    pub vm_rss_kib: Option<u64>,
    pub vm_hwm_kib: Option<u64>,
    pub vm_size_kib: Option<u64>,
    pub threads: Option<u64>,
}

impl ProcStatus {
    pub fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            let mut it = line.split_whitespace();
            let (Some(k), Some(v)) = (it.next(), it.next()) else {
                continue;
            };
            let v = v.parse().ok();
            match k {
                "VmRSS:" => s.vm_rss_kib = v,
                "VmHWM:" => s.vm_hwm_kib = v,
                "VmSize:" => s.vm_size_kib = v,
                "Threads:" => s.threads = v,
                _ => {}
            }
        }
        s
    }

    pub fn read_self() -> Option<Self> {
        Self::read_pid_path(Path::new("/proc/self/status"))
    }

    pub fn read_pid(pid: u32) -> Option<Self> {
        Self::read_pid_path(&PathBuf::from(format!("/proc/{pid}/status")))
    }

    fn read_pid_path(p: &Path) -> Option<Self> {
        std::fs::read_to_string(p).ok().map(|t| Self::parse(&t))
    }
}

/// The four timestamps of spec §26.6 (relative to the touch event).
#[derive(Debug, Clone, Copy, Default)]
pub struct StageTimings {
    pub request_start: Option<Duration>,
    pub decode_done: Option<Duration>,
    pub presented: Option<Duration>,
}

/// Simple stopwatch for stage timings.
#[derive(Debug, Clone, Copy)]
pub struct Stopwatch(Instant);

impl Stopwatch {
    pub fn start() -> Self {
        Self(Instant::now())
    }
    pub fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
    pub fn elapsed_ms(&self) -> u64 {
        self.0.elapsed().as_millis() as u64
    }
}

/// Median / P95 of a sample of milliseconds.
pub fn percentiles(samples: &[u64]) -> Option<(u64, u64)> {
    if samples.is_empty() {
        return None;
    }
    let mut v = samples.to_vec();
    v.sort_unstable();
    let p = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
    Some((p(0.5), p(0.95)))
}

/// Append-only log that rotates once to `<name>.1` when it exceeds `max_bytes`,
/// so flash storage never fills up with debug output.
pub struct BoundedLog {
    path: PathBuf,
    max_bytes: u64,
    file: Option<File>,
    written: u64,
}

impl BoundedLog {
    pub fn open(path: impl Into<PathBuf>, max_bytes: u64) -> Self {
        let path = path.into();
        let (file, written) = match OpenOptions::new().create(true).append(true).open(&path) {
            Ok(f) => {
                let len = f.metadata().map(|m| m.len()).unwrap_or(0);
                (Some(f), len)
            }
            Err(_) => (None, 0),
        };
        Self {
            path,
            max_bytes,
            file,
            written,
        }
    }

    /// A log that discards everything (diagnostics disabled).
    pub fn disabled() -> Self {
        Self {
            path: PathBuf::new(),
            max_bytes: 0,
            file: None,
            written: 0,
        }
    }

    pub fn line(&mut self, msg: &str) {
        if self.file.is_none() {
            return;
        }
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let line = format!("{ts} {msg}\n");
        if self.written + line.len() as u64 > self.max_bytes {
            let rotated = self.path.with_extension("log.1");
            let _ = std::fs::rename(&self.path, rotated);
            self.file = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&self.path)
                .ok();
            self.written = 0;
        }
        if let Some(f) = self.file.as_mut()
            && f.write_all(line.as_bytes()).is_ok()
        {
            self.written += line.len() as u64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status() {
        let s = ProcStatus::parse("Name:\tx\nVmHWM:\t  2048 kB\nVmRSS:\t  1024 kB\nThreads:\t3\n");
        assert_eq!(s.vm_rss_kib, Some(1024));
        assert_eq!(s.vm_hwm_kib, Some(2048));
        assert_eq!(s.threads, Some(3));
        assert_eq!(s.vm_size_kib, None);
    }

    #[test]
    fn percentile_math() {
        assert_eq!(percentiles(&[]), None);
        let v: Vec<u64> = (1..=100).collect();
        assert_eq!(percentiles(&v), Some((51, 95)));
    }

    #[test]
    fn log_is_bounded() {
        let dir = std::env::temp_dir().join(format!("eink-log-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.log");
        let mut log = BoundedLog::open(&p, 200);
        for i in 0..100 {
            log.line(&format!("line {i}"));
        }
        assert!(std::fs::metadata(&p).unwrap().len() <= 200);
        assert!(std::fs::metadata(p.with_extension("log.1")).unwrap().len() <= 200);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
