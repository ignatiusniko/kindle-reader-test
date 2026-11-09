use super::*;
use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;

fn build(entries: &[(&str, &[u8], CompressionMethod)]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data, method) in entries {
        if name.ends_with('/') {
            w.add_directory(*name, SimpleFileOptions::default())
                .unwrap();
            continue;
        }
        w.start_file(
            *name,
            SimpleFileOptions::default().compression_method(*method),
        )
        .unwrap();
        w.write_all(data).unwrap();
    }
    w.finish().unwrap().into_inner()
}

fn open(bytes: Vec<u8>, limits: Limits) -> Result<CbzBook<Cursor<Vec<u8>>>, CbzError> {
    CbzBook::from_reader(Cursor::new(bytes), limits)
}

const S: CompressionMethod = CompressionMethod::Stored;
const D: CompressionMethod = CompressionMethod::Deflated;

#[test]
fn indexes_and_sorts_naturally() {
    let zip = build(&[
        ("pages/", b"", S),
        ("pages/10.jpg", b"ten", D),
        ("pages/2.jpg", b"two", S),
        ("pages/1.PNG", b"one", D),
        ("notes.txt", b"x", D),
        ("__MACOSX/pages/._1.jpg", b"x", D),
        (".hidden.jpg", b"x", D),
    ]);
    let mut book = open(zip, Limits::default()).unwrap();
    let names: Vec<_> = book.pages().iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["pages/1.PNG", "pages/2.jpg", "pages/10.jpg"]);
    assert_eq!(book.pages()[0].kind, PageKind::Png);
    assert_eq!(book.read_page(2).unwrap().1, b"ten");
    assert!(matches!(
        book.read_page(3),
        Err(CbzError::PageOutOfRange {
            page: 3,
            page_count: 3
        })
    ));
    assert_eq!(book.index().skipped.len(), 4);
}

#[test]
fn indexing_many_entries_reads_metadata_only() {
    // 1,200 entries whose data would be invalid images: indexing must not care.
    let names: Vec<String> = (0..1200).map(|i| format!("{i}.jpg")).collect();
    let entries: Vec<(&str, &[u8], CompressionMethod)> = names
        .iter()
        .map(|n| (n.as_str(), &b"not really a jpeg"[..], D))
        .collect();
    let book = open(build(&entries), Limits::default()).unwrap();
    assert_eq!(book.page_count(), 1200);
    assert_eq!(book.pages()[9].name, "9.jpg");
    assert_eq!(book.pages()[10].name, "10.jpg");
}

#[test]
fn entry_count_limit() {
    let entries: Vec<(String, Vec<u8>)> = (0..20).map(|i| (format!("{i}.jpg"), vec![1])).collect();
    let refs: Vec<_> = entries
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_slice(), S))
        .collect();
    let limits = Limits {
        max_archive_entries: 10,
        ..Limits::default()
    };
    assert!(matches!(
        open(build(&refs), limits),
        Err(CbzError::TooManyEntries { .. })
    ));
}

#[test]
fn empty_and_imageless_archives() {
    assert!(matches!(
        open(build(&[]), Limits::default()),
        Err(CbzError::NoReadablePages)
    ));
    assert!(matches!(
        open(build(&[("a.txt", b"x", S)]), Limits::default()),
        Err(CbzError::NoReadablePages)
    ));
}

#[test]
fn not_a_zip_and_truncated_zip() {
    assert!(matches!(
        open(b"definitely not a zip".to_vec(), Limits::default()),
        Err(CbzError::InvalidArchive(_))
    ));
    let mut zip = build(&[("1.jpg", b"abc", S), ("2.jpg", b"def", S)]);
    zip.truncate(zip.len() - 30); // chop the central directory
    assert!(open(zip, Limits::default()).is_err());
}

#[test]
fn oversized_entry_refused_before_decompression() {
    let big = vec![0u8; 4096];
    let limits = Limits {
        uncompressed_entry_limit: 1024,
        ..Limits::default()
    };
    let mut book = open(build(&[("1.jpg", &big, S)]), limits).unwrap();
    assert!(matches!(
        book.read_page(0),
        Err(CbzError::EntryTooLarge { declared: 4096, .. })
    ));
}

#[test]
fn decompression_bomb_ratio_rejected() {
    let zeros = vec![0u8; 2 * 1024 * 1024];
    let mut book = open(build(&[("bomb.png", &zeros, D)]), Limits::default()).unwrap();
    assert!(book.pages()[0].compressed_size < 100_000);
    assert!(matches!(
        book.read_page(0),
        Err(CbzError::SuspiciousRatio { .. })
    ));
}

#[test]
fn path_traversal_names_are_inert() {
    let mut book = open(
        build(&[("../../etc/evil.jpg", b"x", S), ("/abs/2.jpg", b"y", S)]),
        Limits::default(),
    )
    .unwrap();
    // Names are only sort keys; reading returns bytes and writes nothing.
    // The zip crate may refuse to decode unsafe names, in which case the
    // entry is skipped rather than failing the whole book.
    assert!(book.page_count() >= 1);
    assert_eq!(book.page_count() + book.index().skipped.len(), 2);
    assert_eq!(book.read_page(0).unwrap().1.len(), 1);
    assert!(!Path::new("/etc/evil.jpg").exists());
}

#[test]
fn corrupt_deflate_stream_is_an_error_not_a_panic() {
    let data: Vec<u8> = (0..50_000u32).map(|i| (i * 7 % 251) as u8).collect();
    let mut zip = build(&[("1.jpg", &data, D)]);
    // Corrupt bytes in the middle of the compressed stream (after local header).
    for b in &mut zip[100..400] {
        *b ^= 0x5a;
    }
    if let Ok(mut book) = open(zip, Limits::default()) {
        assert!(book.read_page(0).is_err());
    }
}

#[test]
fn missing_file_is_io_not_found() {
    let err = CbzBook::open(Path::new("/nonexistent/book.cbz"), Limits::default())
        .err()
        .unwrap();
    assert!(matches!(&err, CbzError::Io(e) if e.kind() == std::io::ErrorKind::NotFound));
    assert_eq!(err.to_string(), "file not found");
}

#[test]
fn unsupported_compression_is_skipped() {
    // Hand-patch the method field (offset 10 in local header, 10 in central dir).
    let mut zip = build(&[("1.jpg", b"abc", S), ("2.jpg", b"def", S)]);
    let cd = zip
        .windows(4)
        .position(|w| w == [0x50, 0x4b, 0x01, 0x02])
        .unwrap();
    zip[cd + 10] = 14; // LZMA
    let book = open(zip, Limits::default()).unwrap();
    assert_eq!(book.page_count(), 1);
    assert!(matches!(
        book.index().skipped[0].1,
        SkipReason::UnsupportedCompression(_)
    ));
}
