use super::*;
use crate::render::{BookError, BookInfo, RenderedPage};
use std::path::Path;
use std::time::Duration;

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("eink-ctl-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn controller(tag: &str) -> (Controller, PathBuf) {
    let dir = tmp(tag);
    let (store, _) = Store::open(dir.join("data")).unwrap();
    let mut c = Controller::new(
        store,
        Viewport {
            width: 1000,
            height: 1400,
        },
    );
    c.sync_catalog(&[CatalogEntry {
        id: "b1".into(),
        path: "/books/one.cbz".into(),
        title: "One".into(),
        size: 1,
        mtime: 1,
    }]);
    (c, dir)
}

fn submitted(out: &[Output]) -> Option<&Job> {
    out.iter().find_map(|o| match o {
        Output::Submit(j) => Some(j),
        _ => None,
    })
}

fn ready(generation_id: u64, page: u32, w: u32, h: u32) -> Event {
    Event::Page {
        generation: generation_id,
        page_index: page,
        result: PageResult::Ready(RenderedPage {
            image: GrayImage::from_pixels(w, h, vec![255; (w * h) as usize]).unwrap(),
            generation: generation_id,
            page_index: page,
            decode_ms: 3,
            reduced_quality: false,
        }),
        elapsed_ms: 5,
    }
}

/// Open the book and deliver the first page; returns the render generation.
fn open_and_present(c: &mut Controller, pages: u32) -> u64 {
    let out = c.open("b1");
    let generation_id = submitted(&out).unwrap().generation();
    let out = c.on_event(Event::Opened {
        generation: generation_id,
        path: "/books/one.cbz".into(),
        result: Ok(BookInfo {
            page_count: pages,
            skipped_entries: 0,
        }),
    });
    let Job::Render(r) = submitted(&out).unwrap() else {
        panic!()
    };
    let g = r.generation;
    let page = r.page_index;
    let out = c.on_event(ready(g, page, 100, 140));
    assert!(out.iter().any(|o| matches!(o, Output::Present { .. })));
    g
}

#[test]
fn first_open_then_resume_in_one_step() {
    let (mut c, dir) = controller("resume");
    assert_eq!(c.library().continue_reading, None);
    assert_eq!(c.library().books[0].progress_text(), "Not started");
    open_and_present(&mut c, 10);
    // RTL default: left edge = next.
    let t = Instant::now();
    let out = c.tap(10.0, t);
    let Job::Render(r) = submitted(&out).unwrap() else {
        panic!()
    };
    assert_eq!(r.page_index, 1);
    c.on_event(ready(r.generation, 1, 100, 140));
    let _ = c.close();
    c.store_mut().flush().unwrap();

    // "Restart": a new controller sees page 2/10 in Continue.
    let (store, _) = Store::open(dir.join("data")).unwrap();
    let mut c2 = Controller::new(
        store,
        Viewport {
            width: 1000,
            height: 1400,
        },
    );
    c2.sync_catalog(&[CatalogEntry {
        id: "b1".into(),
        path: "/books/one.cbz".into(),
        title: "One".into(),
        size: 1,
        mtime: 1,
    }]);
    let cont = c2.library().continue_reading.unwrap();
    assert_eq!(cont.progress_text(), "Page 2 / 10");
    // One tap: open → resumes at saved page.
    let out = c2.open(&cont.id);
    let generation_id = submitted(&out).unwrap().generation();
    let out = c2.on_event(Event::Opened {
        generation: generation_id,
        path: "/books/one.cbz".into(),
        result: Ok(BookInfo {
            page_count: 10,
            skipped_entries: 0,
        }),
    });
    let Job::Render(r) = submitted(&out).unwrap() else {
        panic!()
    };
    assert_eq!(r.page_index, 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn stale_results_are_ignored() {
    let (mut c, dir) = controller("stale");
    let g = open_and_present(&mut c, 10);
    let t = Instant::now();
    let out1 = c.tap(990.0, t + Duration::from_secs(1)); // RTL: right = previous → AtStart
    assert!(matches!(out1[0], Output::Notice(_)));
    let out = c.next_page();
    let Job::Render(r1) = submitted(&out).unwrap().clone() else {
        panic!()
    };
    // State is busy → second tap is ignored.
    assert!(c.tap(10.0, t + Duration::from_secs(5)).is_empty());
    let out = c.jump_to(5);
    let Job::Render(r2) = submitted(&out).unwrap().clone() else {
        panic!()
    };
    assert!(
        c.on_event(ready(r1.generation, 1, 10, 10)).is_empty(),
        "old generation dropped"
    );
    assert!(c.on_event(ready(g, 0, 10, 10)).is_empty());
    assert!(!c.on_event(ready(r2.generation, 5, 10, 10)).is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn fit_width_reads_tall_pages_in_segments() {
    let (mut c, dir) = controller("segments");
    open_and_present(&mut c, 3);
    let out = c.set_fit(Fit::Width);
    let Job::Render(r) = submitted(&out).unwrap().clone() else {
        panic!()
    };
    // 500×1400 image at width 1000 → 2800 px tall on a 1400 px screen.
    let out = c.on_event(ready(r.generation, 0, 500, 1400));
    let Some(Output::Present {
        display_height,
        offset_px,
        ..
    }) = out.first()
    else {
        panic!()
    };
    assert_eq!((*display_height, *offset_px), (2800.0, 0.0));
    let out = c.next_page();
    assert!(matches!(out[0], Output::Scroll { offset_px } if offset_px > 0.0));
    let out = c.next_page();
    assert!(matches!(out[0], Output::Scroll { offset_px } if offset_px == 1400.0));
    let out = c.next_page();
    assert!(matches!(submitted(&out), Some(Job::Render(r)) if r.page_index == 1));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn errors_offer_skip_and_back() {
    let (mut c, dir) = controller("errors");
    open_and_present(&mut c, 3);
    let out = c.next_page();
    let Job::Render(r) = submitted(&out).unwrap().clone() else {
        panic!()
    };
    let out = c.on_event(Event::Page {
        generation: r.generation,
        page_index: 1,
        result: PageResult::TooLargeForBudget("x".into()),
        elapsed_ms: 1,
    });
    let Output::ShowError(e) = &out[0] else {
        panic!()
    };
    assert_eq!(e.message, MSG_TOO_LARGE);
    assert!(e.can_skip && e.can_retry);
    assert_eq!(c.state(), ReaderState::RecoverableError);
    let out = c.retry();
    assert!(
        matches!(submitted(&out), Some(Job::Render(r)) if r.render.quality == Quality::LowMemory)
    );
    let out = c.skip_page();
    assert!(matches!(submitted(&out), Some(Job::Render(r)) if r.page_index == 2));
    let out = c.close();
    assert!(out.iter().any(|o| matches!(o, Output::ToLibrary)));
    assert_eq!(c.state(), ReaderState::Idle);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn missing_book_keeps_progress() {
    let (mut c, dir) = controller("missing");
    open_and_present(&mut c, 4);
    c.close();
    // Book disappears from the folder.
    c.sync_catalog(&[]);
    let lib = c.library();
    assert!(lib.books[0].missing);
    assert_eq!(
        lib.books[0].progress_text(),
        "File not found — progress kept"
    );
    let out = c.open("b1");
    let generation_id = submitted(&out).unwrap().generation();
    let out = c.on_event(Event::Opened {
        generation: generation_id,
        path: "/books/one.cbz".into(),
        result: Err(BookError {
            kind: BookErrorKind::NotFound,
            message: "file not found".into(),
        }),
    });
    let Output::ShowError(e) = &out[0] else {
        panic!()
    };
    assert_eq!(e.message, MSG_NOT_FOUND);
    assert!(c.store().book("b1").unwrap().last_opened_at > 0);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn modified_file_relinks_progress() {
    let (mut c, dir) = controller("relink");
    open_and_present(&mut c, 4);
    c.close();
    c.sync_catalog(&[CatalogEntry {
        id: "b1-v2".into(),
        path: "/books/one.cbz".into(),
        title: "One".into(),
        size: 2,
        mtime: 9,
    }]);
    assert!(c.store().book("b1").is_none());
    assert!(c.store().book("b1-v2").unwrap().last_opened_at > 0);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn direction_is_per_book_and_changes_tap_meaning() {
    let (mut c, dir) = controller("direction");
    open_and_present(&mut c, 5);
    c.set_direction(Direction::LeftToRight);
    let out = c.tap(990.0, Instant::now() + Duration::from_secs(10));
    assert!(matches!(submitted(&out), Some(Job::Render(r)) if r.page_index == 1));
    assert_eq!(
        c.store().book("b1").unwrap().prefs.direction,
        Direction::LeftToRight
    );
    assert!(matches!(
        c.tap(500.0, Instant::now())[0],
        Output::ShowOptions(true)
    ));
    let _ = Path::new("");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn render_scale_decodes_physical_but_lays_out_logical() {
    let (mut c, dir) = controller("scale");
    c.set_viewport(Viewport {
        width: 500,
        height: 700,
    });
    c.set_render_scale(2.0);
    let out = c.open("b1");
    let g = submitted(&out).unwrap().generation();
    let out = c.on_event(Event::Opened {
        generation: g,
        path: "/books/one.cbz".into(),
        result: Ok(BookInfo {
            page_count: 3,
            skipped_entries: 0,
        }),
    });
    let Job::Render(r) = submitted(&out).unwrap().clone() else {
        panic!()
    };
    assert_eq!(
        (r.render.viewport_width, r.render.viewport_height),
        (1000, 1400)
    );
    let out = c.set_fit(Fit::Width);
    let Job::Render(r) = submitted(&out).unwrap().clone() else {
        panic!()
    };
    // 1000×2800 physical image → 500 logical wide → 1400 logical tall.
    let out = c.on_event(ready(r.generation, r.page_index, 1000, 2800));
    let Some(Output::Present { display_height, .. }) = out.first() else {
        panic!()
    };
    assert_eq!(*display_height, 1400.0);
    // Tap at the logical right edge (LTR) advances a segment, not options.
    c.set_direction(Direction::LeftToRight);
    let out = c.tap(495.0, Instant::now() + Duration::from_secs(10));
    assert!(matches!(out[0], Output::Scroll { .. }), "{out:?}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn refused_display_retries_once_then_errors() {
    let (mut c, dir) = controller("refused");
    open_and_present(&mut c, 3);
    let out = c.present_refused(false);
    assert!(
        matches!(submitted(&out), Some(Job::Render(r)) if r.render.quality == Quality::LowMemory)
    );
    let out = c.present_refused(true);
    let Output::ShowError(e) = &out[0] else {
        panic!("{out:?}")
    };
    assert_eq!(e.message, MSG_TOO_LARGE);
    assert!(e.can_skip && !e.can_retry);
    std::fs::remove_dir_all(dir).unwrap();
}
