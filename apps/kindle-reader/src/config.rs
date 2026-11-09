//! Command-line configuration. Kept dependency-free on purpose.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerMode {
    /// Decode in a background thread (Milestone A; no crash isolation).
    Thread,
    /// Decode in a separate `render-worker` process (Milestone B).
    Process,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Run,
    /// Render UI screens to PNG files without any display (host only).
    Snapshot {
        out: PathBuf,
        font: Option<PathBuf>,
    },
}

#[derive(Debug, Clone)]
pub struct Config {
    pub command: Command,
    pub books_dir: PathBuf,
    pub data_dir: PathBuf,
    pub worker: WorkerMode,
    pub worker_exe: PathBuf,
    /// Hard address-space limit for the worker, bytes. 0 = none.
    pub worker_as_limit: usize,
    pub dpi: f32,
    pub viewport: Option<(u32, u32)>,
    pub log: bool,
    pub desktop: bool,
}

pub const USAGE: &str = "\
eink-reader — minimal CBZ reader for E-Ink

USAGE:
    eink-reader [OPTIONS]
    eink-reader snapshot <OUT_DIR> [--font FILE] [OPTIONS]

OPTIONS:
    --books DIR              Folder scanned for .cbz files (read-only)
                             [default: /mnt/us/documents if present, else ./books]
    --data DIR               App state folder [default: <exe dir>/data]
    --worker thread|process  Where pages are decoded [default: process if
                             render-worker is next to the binary, else thread]
    --worker-exe FILE        Path to render-worker
    --worker-as-limit BYTES  RLIMIT_AS for the worker (0 = off)
    --dpi N                  Screen density for touch-target sizing [default: 300]
    --size WxH               Override viewport size (desktop/snapshot)
    --log                    Write a size-bounded debug log to <data>/logs
    --desktop                Use the desktop preview window even if built with `kindle`
    -h, --help               Show this help
";

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Config, String> {
    let mut args = args.into_iter().peekable();
    let mut command = Command::Run;
    if args.peek().map(String::as_str) == Some("snapshot") {
        args.next();
        let out = args.next().ok_or("snapshot needs an output directory")?;
        command = Command::Snapshot {
            out: out.into(),
            font: None,
        };
    }
    let default_books = if Path::new("/mnt/us/documents").is_dir() {
        PathBuf::from("/mnt/us/documents")
    } else {
        PathBuf::from("books")
    };
    let mut cfg = Config {
        command,
        books_dir: default_books,
        data_dir: exe_dir().join("data"),
        worker: WorkerMode::Thread,
        worker_exe: exe_dir().join("render-worker"),
        worker_as_limit: usize::MAX,
        dpi: 300.0,
        viewport: None,
        log: false,
        desktop: false,
    };
    let mut worker: Option<WorkerMode> = None;
    while let Some(a) = args.next() {
        let mut val = |name: &str| args.next().ok_or(format!("{name} needs a value"));
        match a.as_str() {
            "--books" => cfg.books_dir = val("--books")?.into(),
            "--data" => cfg.data_dir = val("--data")?.into(),
            "--worker" => {
                worker = Some(match val("--worker")?.as_str() {
                    "thread" => WorkerMode::Thread,
                    "process" => WorkerMode::Process,
                    other => return Err(format!("unknown worker mode {other}")),
                })
            }
            "--worker-exe" => cfg.worker_exe = val("--worker-exe")?.into(),
            "--worker-as-limit" => {
                cfg.worker_as_limit = val("--worker-as-limit")?
                    .parse()
                    .map_err(|_| "invalid --worker-as-limit")?
            }
            "--dpi" => {
                cfg.dpi = val("--dpi")?.parse().map_err(|_| "invalid --dpi")?;
                if !(50.0..=1200.0).contains(&cfg.dpi) {
                    return Err("--dpi out of range".into());
                }
            }
            "--size" => {
                let v = val("--size")?;
                let (w, h) = v.split_once('x').ok_or("--size must be WxH")?;
                cfg.viewport = Some((
                    w.parse().map_err(|_| "invalid width")?,
                    h.parse().map_err(|_| "invalid height")?,
                ));
            }
            "--font" => {
                let f = val("--font")?;
                match &mut cfg.command {
                    Command::Snapshot { font, .. } => *font = Some(f.into()),
                    Command::Run => return Err("--font is only for snapshot".into()),
                }
            }
            "--log" => cfg.log = true,
            "--desktop" => cfg.desktop = true,
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    cfg.worker = worker.unwrap_or(if cfg.worker_exe.is_file() {
        WorkerMode::Process
    } else {
        WorkerMode::Thread
    });
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(a: &[&str]) -> Result<Config, String> {
        parse(a.iter().map(|s| s.to_string()))
    }

    #[test]
    fn parses_options() {
        let c = p(&[
            "--books", "/b", "--worker", "thread", "--dpi", "212", "--size", "800x600",
        ])
        .unwrap();
        assert_eq!(c.books_dir, PathBuf::from("/b"));
        assert_eq!(c.worker, WorkerMode::Thread);
        assert_eq!(c.viewport, Some((800, 600)));
        assert_eq!(c.command, Command::Run);
        let c = p(&["snapshot", "out", "--font", "f.ttf"]).unwrap();
        assert!(matches!(c.command, Command::Snapshot { font: Some(_), .. }));
        assert!(p(&["--bogus"]).is_err());
        assert!(p(&["--dpi", "5"]).is_err());
        assert!(p(&["--font", "x"]).is_err());
    }
}
