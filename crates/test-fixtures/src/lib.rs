//! Synthetic CBZ fixtures (spec §26.2), generated in Rust so tests and CI
//! need neither Python nor Pillow. All images are drawn here; nothing is
//! copied from real books.

use std::io::{Cursor, Write};

use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

/// Draw a test page: white background, thick border, page number as big
/// blocky digits, and a grey ramp at the bottom (E-Ink grayscale check).
pub fn page_pixels(w: u32, h: u32, number: u32) -> Vec<u8> {
    let mut px = vec![255u8; (w * h) as usize];
    let border = (w.min(h) / 60).max(2);
    let margin = w.min(h) / 20;
    for y in 0..h {
        for x in 0..w {
            let inside = x >= margin && x < w - margin && y >= margin && y < h - margin;
            let edge = inside
                && (x < margin + border
                    || x >= w - margin - border
                    || y < margin + border
                    || y >= h - margin - border);
            if edge {
                px[(y * w + x) as usize] = 0;
            }
        }
    }
    // Grey ramp: 16 steps across the bottom band.
    let ramp_top = h - margin - border - h / 10;
    for y in ramp_top..(h - margin - border) {
        for x in (margin + border)..(w - margin - border) {
            let step = (x - margin - border) * 16 / (w - 2 * (margin + border));
            px[(y * w + x) as usize] = (step * 17) as u8;
        }
    }
    draw_number(&mut px, w, h, number);
    px
}

const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b010, 0b010, 0b010],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];

fn draw_number(px: &mut [u8], w: u32, h: u32, n: u32) {
    let s = n.to_string();
    let cell = (w / 30).max(2);
    let total_w = s.len() as u32 * 4 * cell;
    let x0 = w.saturating_sub(total_w) / 2;
    let y0 = h / 3;
    for (i, ch) in s.bytes().enumerate() {
        let glyph = DIGITS[(ch - b'0') as usize];
        for (gy, row) in glyph.iter().enumerate() {
            for gx in 0..3u32 {
                if row & (0b100 >> gx) == 0 {
                    continue;
                }
                for dy in 0..cell {
                    for dx in 0..cell {
                        let x = x0 + i as u32 * 4 * cell + gx * cell + dx;
                        let y = y0 + gy as u32 * cell + dy;
                        if x < w && y < h {
                            px[(y * w + x) as usize] = 0;
                        }
                    }
                }
            }
        }
    }
}

pub fn encode_jpeg(gray: &[u8], w: u32, h: u32, progressive: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = jpeg_encoder::Encoder::new(&mut out, 85);
    enc.set_progressive(progressive);
    enc.encode(gray, w as u16, h as u16, jpeg_encoder::ColorType::Luma)
        .expect("jpeg encode");
    out
}

pub fn encode_png(gray: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, w, h);
        e.set_color(png::ColorType::Grayscale);
        e.set_depth(png::BitDepth::Eight);
        let mut wr = e.write_header().expect("png header");
        wr.write_image_data(gray).expect("png data");
    }
    out
}

/// Build a ZIP from `(name, bytes, deflate?)` entries.
pub fn zip(entries: &[(String, Vec<u8>, bool)]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data, deflate) in entries {
        let method = if *deflate {
            CompressionMethod::Deflated
        } else {
            CompressionMethod::Stored
        };
        w.start_file(
            name.as_str(),
            SimpleFileOptions::default().compression_method(method),
        )
        .expect("zip entry");
        w.write_all(data).expect("zip write");
    }
    w.finish().expect("zip finish").into_inner()
}

/// `pages` JPEG pages of `w×h`, named `pages/001.jpg`… (stored, like most CBZs).
pub fn jpeg_book(pages: u32, w: u32, h: u32) -> Vec<u8> {
    let entries: Vec<_> = (1..=pages)
        .map(|i| {
            let px = page_pixels(w, h, i);
            (
                format!("pages/{i:03}.jpg"),
                encode_jpeg(&px, w, h, false),
                false,
            )
        })
        .collect();
    zip(&entries)
}

pub fn png_book(pages: u32, w: u32, h: u32) -> Vec<u8> {
    let entries: Vec<_> = (1..=pages)
        .map(|i| {
            let px = page_pixels(w, h, i);
            (format!("{i:03}.png"), encode_png(&px, w, h), true)
        })
        .collect();
    zip(&entries)
}

/// A book whose `bad_page` (1-based) is not a valid image.
pub fn corrupt_page_book(pages: u32, bad_page: u32) -> Vec<u8> {
    let entries: Vec<_> = (1..=pages)
        .map(|i| {
            let data = if i == bad_page {
                let mut j = encode_jpeg(&page_pixels(300, 400, i), 300, 400, false);
                let n = j.len();
                j.truncate(n / 4);
                j.extend_from_slice(&[0x00; 64]);
                j
            } else {
                encode_jpeg(&page_pixels(300, 400, i), 300, 400, false)
            };
            (format!("{i:03}.jpg"), data, false)
        })
        .collect();
    zip(&entries)
}

/// Names 1, 2, 10 out of order to test natural sorting.
pub fn rtl_order_book() -> Vec<u8> {
    let mk = |n: u32| encode_jpeg(&page_pixels(200, 300, n), 200, 300, false);
    zip(&[
        ("10.jpg".into(), mk(10), false),
        ("1.jpg".into(), mk(1), false),
        ("2.jpg".into(), mk(2), false),
    ])
}

/// Many tiny entries for indexing tests (decoding is not expected).
pub fn long_index_book(entries: u32) -> Vec<u8> {
    let tiny = encode_jpeg(&[128; 64], 8, 8, false);
    let v: Vec<_> = (1..=entries)
        .map(|i| (format!("p{i:05}.jpg"), tiny.clone(), false))
        .collect();
    zip(&v)
}

/// Single page with large dimensions but a small file.
pub fn oversized_jpeg_book(w: u32, h: u32) -> Vec<u8> {
    let px = vec![255u8; (w * h) as usize];
    zip(&[("huge.jpg".into(), encode_jpeg(&px, w, h, false), false)])
}

pub fn oversized_png_book(w: u32, h: u32) -> Vec<u8> {
    let px = vec![255u8; (w * h) as usize];
    zip(&[("huge.png".into(), encode_png(&px, w, h), true)])
}

/// Highly compressible entry (trips the compression-ratio guard).
pub fn suspicious_ratio_book() -> Vec<u8> {
    let mut data = b"\x89PNG\r\n\x1a\n".to_vec();
    data.resize(8 * 1024 * 1024, 0);
    zip(&[("bomb.png".into(), data, true)])
}

/// Valid book with the central directory chopped off.
pub fn bad_zip() -> Vec<u8> {
    let mut z = jpeg_book(3, 100, 150);
    let n = z.len();
    z.truncate(n - 40);
    z
}
