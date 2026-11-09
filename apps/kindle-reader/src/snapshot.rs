//! Headless UI snapshots (host only, spec §26.1 "Host preview/simulator").
//!
//! Renders the real Slint UI with the software renderer into an in-memory
//! buffer and writes PNGs quantised to 16 grey levels, roughly what the
//! E-Ink panel can show. This checks layout and flows without a device; it
//! does **not** validate waveforms, ghosting, touch mapping or memory.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, PhysicalSize, Rgb8Pixel};

use crate::AppWindow;
use crate::app::{self, SyncSink};
use crate::config::Config;

struct Headless {
    window: Rc<MinimalSoftwareWindow>,
}

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }
}

pub struct Harness {
    pub app: AppWindow,
    window: Rc<MinimalSoftwareWindow>,
    buffer: Vec<Rgb8Pixel>,
    width: u32,
    height: u32,
    events: Rc<RefCell<VecDeque<reader_core::scheduler::Event>>>,
}

fn find_font(explicit: Option<&Path>) -> Option<PathBuf> {
    let candidates = [
        explicit.map(Path::to_path_buf),
        Some(PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/fonts/ReaderFont.ttf"
        ))),
        Some(PathBuf::from(
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        )),
        Some(PathBuf::from("/usr/share/fonts/TTF/DejaVuSans.ttf")),
        Some(PathBuf::from(
            "/System/Library/Fonts/Supplemental/Arial.ttf",
        )),
    ];
    candidates.into_iter().flatten().find(|p| p.is_file())
}

impl Harness {
    /// Must be called once per process (Slint allows one platform).
    pub fn new(cfg: Config, font: Option<&Path>) -> Result<Self, String> {
        if let Some(f) = find_font(font) {
            // SAFETY: called before any other thread reads the environment.
            unsafe { std::env::set_var("SLINT_DEFAULT_FONT", f) };
        }
        let (width, height) = cfg.viewport.unwrap_or((1072, 1448));
        let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        window.set_size(PhysicalSize::new(width, height));
        slint::platform::set_platform(Box::new(Headless {
            window: window.clone(),
        }))
        .map_err(|e| e.to_string())?;
        let app = AppWindow::new().map_err(|e| e.to_string())?;
        let events = Rc::new(RefCell::new(VecDeque::new()));
        let ev = events.clone();
        app::install(&app, cfg, move |_, renderer| {
            Box::new(SyncSink {
                renderer: RefCell::new(renderer),
                events: ev,
            })
        })?;
        app.show().map_err(|e| e.to_string())?;
        app::with_state(|st| st.scan_now());
        Ok(Self {
            app,
            window,
            buffer: vec![Rgb8Pixel::default(); (width * height) as usize],
            width,
            height,
            events,
        })
    }

    /// Deliver queued render events (what the scheduler thread would do).
    pub fn pump(&self) {
        loop {
            let ev = self.events.borrow_mut().pop_front();
            let Some(ev) = ev else { break };
            app::with_state(|st| st.on_scheduler_event(ev));
        }
        slint::platform::update_timers_and_animations();
    }

    /// Render the current frame and return 16-level grey pixels.
    pub fn frame(&mut self) -> Vec<u8> {
        self.pump();
        self.window.request_redraw();
        let (buf, w) = (&mut self.buffer, self.width as usize);
        self.window.draw_if_needed(|r| {
            r.render(buf, w);
        });
        self.buffer
            .iter()
            .map(|p| {
                let l = (77 * p.r as u32 + 150 * p.g as u32 + 29 * p.b as u32 + 128) >> 8;
                ((l + 8) / 17 * 17).min(255) as u8
            })
            .collect()
    }

    pub fn save(&mut self, path: &Path) -> Result<(), String> {
        let px = self.frame();
        let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), self.width, self.height);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().map_err(|e| e.to_string())?;
        w.write_image_data(&px).map_err(|e| e.to_string())?;
        println!("wrote {}", path.display());
        Ok(())
    }
}

/// `eink-reader snapshot OUT_DIR`: library, reader, options, advanced, error.
pub fn run(cfg: Config, out: &Path, font: Option<&Path>) -> Result<(), String> {
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let mut h = Harness::new(cfg, font)?;
    h.save(&out.join("01-library-first-run.png"))?;
    let Some(id) = app::first_book_id() else {
        return Err("no .cbz books found; pass --books DIR with at least one CBZ".into());
    };
    h.app.invoke_open_book(id.into());
    h.save(&out.join("02-reader.png"))?;
    h.app.invoke_page_tap(10.0); // RTL default: left edge = next page
    // Bypass the double-tap gate for the snapshot sequence.
    std::thread::sleep(std::time::Duration::from_millis(350));
    h.save(&out.join("03-reader-page-2.png"))?;
    h.app.invoke_page_tap((h.width / 2) as f32);
    h.save(&out.join("04-reading-options.png"))?;
    h.app.set_advanced_open(true);
    h.save(&out.join("05-reading-options-advanced.png"))?;
    h.app.set_advanced_open(false);
    h.app.set_options_open(false);
    h.app.invoke_set_fit_width(true);
    h.save(&out.join("06-fit-width.png"))?;
    h.app.invoke_to_library();
    h.save(&out.join("07-library-continue.png"))?;
    app::with_state(|st| st.flush());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Command, WorkerMode};
    use slint::Model;

    /// End-to-end UI flow through the real Slint components (headless).
    /// One test only: Slint allows a single platform per process.
    #[test]
    fn ui_flow_library_reader_options_errors() {
        let root = std::env::temp_dir().join(format!("eink-ui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let books = root.join("books");
        std::fs::create_dir_all(&books).unwrap();
        std::fs::write(
            books.join("A_Sample.cbz"),
            test_fixtures::jpeg_book(6, 1200, 1800),
        )
        .unwrap();
        std::fs::write(
            books.join("B_Damaged.cbz"),
            test_fixtures::corrupt_page_book(4, 2),
        )
        .unwrap();
        let cfg = Config {
            command: Command::Run,
            books_dir: books,
            data_dir: root.join("data"),
            worker: WorkerMode::Thread,
            worker_exe: "/nonexistent".into(),
            worker_as_limit: 0,
            dpi: 300.0,
            viewport: Some((1072, 1448)),
            log: true,
            desktop: false,
        };
        let mut h = Harness::new(cfg, None).unwrap();
        let app = h.app.clone_strong();
        let wait_gate = || std::thread::sleep(std::time::Duration::from_millis(320));

        // Library: two books, no "Continue" yet.
        h.frame();
        assert_eq!(app.get_screen(), 0);
        assert!(!app.get_has_continue());
        assert_eq!(app.get_books().row_count(), 2);

        // Open A → page 1.
        let a = app.get_books().row_data(0).unwrap();
        assert_eq!(a.title, "A Sample");
        app.invoke_open_book(a.id.clone());
        let px = h.frame();
        assert_eq!(app.get_screen(), 1);
        assert!(app.get_has_image());
        assert_eq!((app.get_page_number(), app.get_page_count()), (1, 6));
        assert!(px.contains(&0), "page border should be black");

        // RTL default: tap left edge = next page.
        wait_gate();
        app.invoke_page_tap(20.0);
        h.frame();
        assert_eq!(app.get_page_number(), 2);
        // Duplicate tap right after is ignored by the gate.
        app.invoke_page_tap(20.0);
        h.frame();
        assert_eq!(app.get_page_number(), 2);

        // Centre tap → options; switch to LTR; right edge now = next.
        app.invoke_page_tap(536.0);
        h.frame();
        assert!(app.get_options_open());
        app.invoke_set_rtl(false);
        app.set_options_open(false);
        wait_gate();
        app.invoke_page_tap(1060.0);
        h.frame();
        assert_eq!(app.get_page_number(), 3);

        // Fit width: 1200×1800 page is taller than the screen → segments.
        app.invoke_set_fit_width(true);
        h.frame();
        assert!(app.get_fit_width());
        assert!(app.get_page_display_height() > 1448.0);
        assert_eq!(app.get_page_offset(), 0.0);
        app.invoke_next_page();
        h.frame();
        assert!(app.get_page_offset() > 0.0, "scrolled to second segment");
        assert_eq!(app.get_page_number(), 3);

        // Back to library: Continue shows A at page 3.
        app.invoke_to_library();
        h.frame();
        assert_eq!(app.get_screen(), 0);
        assert!(app.get_has_continue());
        assert_eq!(app.get_continue_book().progress, "Page 3 / 6");

        // Damaged book: page 2 shows a recoverable error with Skip.
        let b = app.get_books().row_data(1).unwrap();
        app.invoke_open_book(b.id);
        h.frame();
        app.invoke_jump(0.34); // → page index 1 (page 2)
        h.frame();
        assert!(app.get_error_visible());
        assert!(app.get_error_can_skip());
        app.invoke_skip();
        h.frame();
        assert!(!app.get_error_visible());
        assert_eq!(app.get_page_number(), 3);

        // Progress survives a flush + reload of the store.
        app::with_state(|st| st.flush());
        let (store, _) = persistence::Store::open(root.join("data")).unwrap();
        let rec = store
            .state()
            .books
            .values()
            .find(|b| b.title == "A Sample")
            .unwrap();
        assert_eq!(rec.last_read_page, 2);
        assert_eq!(rec.prefs.fit, persistence::Fit::Width);
        assert_eq!(rec.prefs.direction, persistence::Direction::LeftToRight);
        app::shutdown();
        std::fs::remove_dir_all(root).unwrap();
    }
}
