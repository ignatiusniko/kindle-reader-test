//! E-Ink Reader — Slint shell around `reader-core`.
//!
//! See README.md for the device gates (G0–G7). Building this binary for the
//! Kindle is **not** evidence that the display, touch input or Kindle UI
//! hand-off work on a Kindle Basic 2022; launch it only through a verified,
//! device-specific launcher (docs/safety-and-recovery.md).

// musl's allocator fragments under repeated page decodes (RSS grew without
// bound in MEM-01); mimalloc keeps it flat. See docs/decisions/0003.
#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod app;
mod config;
mod platform;
mod signals;
mod snapshot;

use std::process::ExitCode;

use config::{Command, Config};
use slint::ComponentHandle;

slint::include_modules!();

fn run(cfg: Config) -> Result<(), String> {
    let sigset = signals::block_termination_signals();
    let _backend = platform::install(&cfg)?;
    let app = AppWindow::new().map_err(|e| e.to_string())?;
    app::install(&app, cfg, app::threaded_sink)?;
    signals::spawn_quit_on_signal(sigset);
    app::with_state(|st| st.start_scan());
    let result = app.run().map_err(|e| e.to_string());
    app::shutdown();
    result
}

fn main() -> ExitCode {
    let cfg = match config::parse(std::env::args().skip(1)) {
        Ok(c) => c,
        Err(e) => {
            if e.is_empty() {
                print!("{}", config::USAGE);
                return ExitCode::SUCCESS;
            }
            eprintln!("eink-reader: {e}\n\n{}", config::USAGE);
            return ExitCode::from(2);
        }
    };
    let result = match cfg.command.clone() {
        Command::Run => run(cfg),
        Command::Snapshot { out, font } => snapshot::run(cfg, &out, font.as_deref()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("eink-reader: {e}");
            ExitCode::FAILURE
        }
    }
}
