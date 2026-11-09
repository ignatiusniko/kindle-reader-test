use super::*;
use memory_budget::MIB;

fn jpeg(w: u16, h: u16, progressive: bool) -> Vec<u8> {
    let mut px = Vec::with_capacity(w as usize * h as usize * 3);
    for y in 0..h {
        for x in 0..w {
            let v = if (x / 16 + y / 16) % 2 == 0 { 20 } else { 230 };
            px.extend_from_slice(&[v, v, v]);
        }
    }
    let mut out = Vec::new();
    let mut enc = jpeg_encoder::Encoder::new(&mut out, 85);
    enc.set_progressive(progressive);
    enc.encode(&px, w, h, jpeg_encoder::ColorType::Rgb).unwrap();
    out
}

fn png_bytes(w: u32, h: u32, color: png::ColorType) -> Vec<u8> {
    let ch = color.samples();
    let mut data = vec![0u8; w as usize * h as usize * ch];
    for (i, p) in data.chunks_exact_mut(ch).enumerate() {
        let v = if (i as u32 % w) < w / 2 { 0 } else { 255 };
        p.fill(v);
        if ch == 4 || ch == 2 {
            *p.last_mut().unwrap() = 255;
        }
    }
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, w, h);
        e.set_color(color);
        e.set_depth(png::BitDepth::Eight);
        let mut wr = e.write_header().unwrap();
        wr.write_image_data(&data).unwrap();
    }
    out
}

fn never() -> bool {
    false
}

#[test]
fn sniffing() {
    assert_eq!(sniff(&jpeg(8, 8, false)), Some(ImageKind::Jpeg));
    assert_eq!(
        sniff(&png_bytes(2, 2, png::ColorType::Grayscale)),
        Some(ImageKind::Png)
    );
    assert_eq!(sniff(b"GIF89a"), None);
    assert!(matches!(
        probe(b"GIF89a"),
        Err(RenderError::UnsupportedFormat(_))
    ));
}

#[test]
fn output_size_fit_modes() {
    let l = Limits::default();
    let mut r = RenderRequest::new(1072, 1448);
    // Larger portrait page: fit page limited by height.
    assert_eq!(output_size(2000, 3000, &r, &l), (965, 1448));
    // Small image is never upscaled.
    assert_eq!(output_size(500, 700, &r, &l), (500, 700));
    r.fit = FitMode::Width;
    assert_eq!(output_size(2000, 3000, &r, &l), (1072, 1608));
    r.quality = Quality::LowMemory;
    assert_eq!(output_size(2000, 3000, &r, &l), (536, 804));
    // Very tall strip obeys the output pixel cap.
    r.quality = Quality::Normal;
    let (w, h) = output_size(1000, 30000, &r, &l);
    assert!(w as u64 * h as u64 <= l.max_output_pixels);
}

#[test]
fn jpeg_is_scaled_during_decode() {
    let bytes = jpeg(2400, 3200, false);
    let budget = MemoryBudget::new(64 * MIB);
    let img = render_gray(
        &bytes,
        &RenderRequest::new(1072, 1448),
        &Limits::default(),
        &budget,
        &never,
    )
    .unwrap();
    assert_eq!((img.width, img.height), (1072, 1429));
    // Full-size decode would plan 2 × 23 MB; DCT scaling (1/2 here) keeps
    // the planned peak at roughly a quarter of that.
    assert!(budget.peak() < 16 * MIB, "peak {}", budget.peak());
    // The page buffer stays reserved while the image lives.
    assert_eq!(budget.in_use(), 1072 * 1429);
    drop(img);
    assert_eq!(budget.in_use(), 0);
}

#[test]
fn progressive_jpeg_accounts_for_coefficients() {
    let bytes = jpeg(1600, 2400, true);
    assert!(probe(&bytes).unwrap().needs_full_frame);
    let small = MemoryBudget::new(8 * MIB);
    let r = RenderRequest::new(1072, 1448);
    assert!(matches!(
        render_gray(&bytes, &r, &Limits::default(), &small, &never),
        Err(RenderError::TooLarge(_))
    ));
    assert_eq!(small.in_use(), 0, "refusal must release reservations");
    let big = MemoryBudget::new(64 * MIB);
    assert!(render_gray(&bytes, &r, &Limits::default(), &big, &never).is_ok());
}

#[test]
fn png_streams_rows_with_small_peak() {
    // 2000×8000 RGBA would be 64 MB as a full frame.
    let bytes = png_bytes(2000, 8000, png::ColorType::Rgba);
    let budget = MemoryBudget::new(8 * MIB);
    let img = render_gray(
        &bytes,
        &RenderRequest::new(1072, 1448),
        &Limits::default(),
        &budget,
        &never,
    )
    .unwrap();
    assert_eq!((img.width, img.height), (362, 1448));
    assert!(budget.peak() < MIB, "peak {}", budget.peak());
    // Left half black, right half white.
    assert_eq!(img.pixels[10], 0);
    assert_eq!(img.pixels[350], 255);
}

#[test]
fn png_color_types() {
    for c in [
        png::ColorType::Grayscale,
        png::ColorType::GrayscaleAlpha,
        png::ColorType::Rgb,
    ] {
        let bytes = png_bytes(64, 64, c);
        let img = render_gray(
            &bytes,
            &RenderRequest::new(32, 32),
            &Limits::default(),
            &MemoryBudget::new(MIB),
            &never,
        )
        .unwrap();
        assert_eq!((img.width, img.height), (32, 32));
    }
}

#[test]
fn interlaced_png_needs_full_frame_budget() {
    // Synthetic Adam7 fixture, hand-encoded (png 0.18 cannot write interlaced).
    let bytes = include_bytes!("../testdata/interlaced_1000.png").to_vec();
    assert!(probe(&bytes).unwrap().needs_full_frame);
    let r = RenderRequest::new(500, 500);
    assert!(matches!(
        render_gray(
            &bytes,
            &r,
            &Limits::default(),
            &MemoryBudget::new(2 * MIB),
            &never
        ),
        Err(RenderError::TooLarge(_))
    ));
    let img = render_gray(
        &bytes,
        &r,
        &Limits::default(),
        &MemoryBudget::new(16 * MIB),
        &never,
    )
    .unwrap();
    assert_eq!(img.width, 500);
}

#[test]
fn oversized_dimensions_rejected_by_header() {
    // A tiny PNG claiming 30000×30000 must be refused before decoding.
    let bytes = png_bytes(30000, 1, png::ColorType::Grayscale);
    let limits = Limits {
        max_source_dimension: 20000,
        ..Limits::default()
    };
    assert!(matches!(
        render_gray(
            &bytes,
            &RenderRequest::new(100, 100),
            &limits,
            &MemoryBudget::new(64 * MIB),
            &never
        ),
        Err(RenderError::TooLarge(_))
    ));
    let limits = Limits {
        max_source_pixels: 1000,
        ..Limits::default()
    };
    assert!(matches!(
        render_gray(
            &jpeg(100, 100, false),
            &RenderRequest::new(100, 100),
            &limits,
            &MemoryBudget::new(64 * MIB),
            &never
        ),
        Err(RenderError::TooLarge(_))
    ));
}

#[test]
fn corrupt_inputs_do_not_panic() {
    let budget = MemoryBudget::new(64 * MIB);
    let r = RenderRequest::new(200, 200);
    let mut j = jpeg(300, 300, false);
    j.truncate(j.len() / 3);
    let _ = render_gray(&j, &r, &Limits::default(), &budget, &never);
    let mut p = png_bytes(300, 300, png::ColorType::Rgb);
    let n = p.len();
    for b in &mut p[n / 2..n - 20] {
        *b = 0xAB;
    }
    assert!(render_gray(&p, &r, &Limits::default(), &budget, &never).is_err());
    // Garbage after valid magic bytes.
    let mut g = b"\x89PNG\r\n\x1a\n".to_vec();
    g.extend_from_slice(&[0xff; 64]);
    assert!(render_gray(&g, &r, &Limits::default(), &budget, &never).is_err());
    let mut g = vec![0xff, 0xd8, 0xff];
    g.extend_from_slice(&[0x00; 64]);
    assert!(render_gray(&g, &r, &Limits::default(), &budget, &never).is_err());
    assert_eq!(budget.in_use(), 0);
}

#[test]
fn cancellation_releases_budget() {
    let bytes = png_bytes(500, 4000, png::ColorType::Rgb);
    let budget = MemoryBudget::new(64 * MIB);
    let r = render_gray(
        &bytes,
        &RenderRequest::new(100, 100),
        &Limits::default(),
        &budget,
        &|| true,
    );
    assert!(matches!(r, Err(RenderError::Cancelled)));
    assert_eq!(budget.in_use(), 0);
}

#[test]
fn darker_contrast_darkens_midtones() {
    let mut px = vec![0, 128, 255];
    apply_contrast(&mut px, Contrast::Darker);
    assert_eq!(px[0], 0);
    assert!(px[1] < 128);
    assert_eq!(px[2], 255);
}
