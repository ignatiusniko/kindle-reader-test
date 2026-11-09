//! Host versions of CBZ-01/03 and MEM-01 (spec §26.4). These prove the
//! *logic* is bounded; real numbers must still be measured on the Kindle.

use std::path::PathBuf;
use std::time::Duration;

use image_pipeline::RenderRequest;
use memory_budget::{Limits, MIB, MemoryBudget};
use reader_core::render::{InProcessRenderer, PageRenderer, PageRequest, PageResult};

fn write(tag: &str, bytes: Vec<u8>) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("eink-host-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(format!("{tag}.cbz"));
    std::fs::write(&p, bytes).unwrap();
    p
}

#[test]
fn cbz01_all_pages_of_sample_render() {
    let book = write("sample", test_fixtures::jpeg_book(12, 1200, 1800));
    let before = std::fs::read(&book).unwrap();
    let mut r = InProcessRenderer::new(Limits::default(), MemoryBudget::new(64 * MIB));
    assert_eq!(r.open(&book).unwrap().page_count, 12);
    for page in 0..12 {
        let req = PageRequest {
            book_path: book.clone(),
            page_index: page,
            render: RenderRequest::new(1072, 1448),
            generation: 1,
        };
        let PageResult::Ready(p) = r.render(&req, &|| false) else {
            panic!("page {page}")
        };
        assert_eq!((p.image.width, p.image.height), (965, 1448));
    }
    assert_eq!(
        std::fs::read(&book).unwrap(),
        before,
        "book must be unchanged"
    );
}

#[test]
fn cbz03_thousand_entry_index_is_metadata_only() {
    let book = write("long", test_fixtures::long_index_book(1500));
    let budget = MemoryBudget::new(64 * MIB);
    let mut r = InProcessRenderer::new(Limits::default(), budget.clone());
    let t = std::time::Instant::now();
    assert_eq!(r.open(&book).unwrap().page_count, 1500);
    assert_eq!(budget.peak(), 0, "indexing must not reserve decode memory");
    assert!(t.elapsed() < Duration::from_secs(5));
}

#[test]
fn rtl_order_is_natural() {
    let book = write("order", test_fixtures::rtl_order_book());
    let b = cbz_reader_names(&book);
    assert_eq!(b, ["1.jpg", "2.jpg", "10.jpg"]);
}

fn cbz_reader_names(p: &std::path::Path) -> Vec<String> {
    // Indirectly via the renderer API: page sizes differ per name order.
    let book = cbz_reader::CbzBook::open(p, Limits::default()).unwrap();
    book.pages().iter().map(|p| p.name.clone()).collect()
}

#[test]
fn err01_bad_inputs_fail_gracefully() {
    let mut r = InProcessRenderer::new(Limits::default(), MemoryBudget::new(64 * MIB));
    assert!(r.open(&write("badzip", test_fixtures::bad_zip())).is_err());
    let bomb = write("bomb", test_fixtures::suspicious_ratio_book());
    r.open(&bomb).unwrap();
    let req = |p: &PathBuf, page| PageRequest {
        book_path: p.clone(),
        page_index: page,
        render: RenderRequest::new(1072, 1448),
        generation: 1,
    };
    assert!(matches!(
        r.render(&req(&bomb, 0), &|| false),
        PageResult::TooLargeForBudget(_)
    ));
    let corrupt = write("corrupt", test_fixtures::corrupt_page_book(3, 2));
    assert!(matches!(
        r.render(&req(&corrupt, 1), &|| false),
        PageResult::CorruptPage(_)
    ));
    assert!(matches!(
        r.render(&req(&corrupt, 2), &|| false),
        PageResult::Ready(_)
    ));
}

#[test]
fn mem02_huge_png_streams_or_refuses() {
    let book = write("hugepng", test_fixtures::oversized_png_book(4000, 12000));
    let budget = MemoryBudget::new(16 * MIB);
    let mut r = InProcessRenderer::new(Limits::default(), budget.clone());
    let req = PageRequest {
        book_path: book,
        page_index: 0,
        render: RenderRequest::new(1072, 1448),
        generation: 1,
    };
    match r.render(&req, &|| false) {
        PageResult::Ready(p) => assert!(p.image.width <= 1072 && p.image.height <= 1448),
        PageResult::TooLargeForBudget(_) => {}
        other => panic!("{other:?}"),
    }
    // 48 MP Gray would be 48 MB as a full frame; streaming never got close.
    assert!(budget.peak() < 8 * MIB, "peak {}", budget.peak());
}
