//! Bounded page rendering: compressed JPEG/PNG bytes → display-sized Gray8.
//!
//! Rules from spec §5 / §24.5 that this crate enforces:
//!
//! * Headers are inspected first; dimensions are validated against
//!   [`Limits`] with checked arithmetic before anything large is allocated.
//! * Every large allocation is preceded by a [`MemoryBudget`] reservation.
//! * **JPEG** uses decoder-assisted DCT scaling (1/2, 1/4, 1/8) so a huge
//!   source never materialises at full resolution (baseline JPEG). Progressive
//!   JPEGs keep full-size coefficients, which is accounted for and may be
//!   refused.
//! * **PNG** (non-interlaced) is decoded **row by row** and box-filtered into
//!   the output, so peak memory is ~one source row + the output buffer,
//!   independent of source height. Interlaced PNG needs a full frame and is
//!   only decoded if that fits the budget.
//! * Output is never larger than the source in either axis: upscaling is left
//!   to the display layer, which avoids allocating oversize rasters for small
//!   images.

use std::fmt;
use std::io::Cursor;

use memory_budget::{BudgetExceeded, Limits, MemoryBudget, Reservation, required_bytes};

mod downscale;

pub use downscale::AreaDownscaler;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FitMode {
    /// Whole page visible (default).
    #[default]
    Page,
    /// Page width fills the screen; tall pages are read in vertical segments.
    Width,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Quality {
    #[default]
    Normal,
    /// Retry mode after a budget/worker failure: half resolution per axis.
    LowMemory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Contrast {
    #[default]
    Normal,
    /// Darkens mid-tones; helps faint scans on E-Ink.
    Darker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderRequest {
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub fit: FitMode,
    pub quality: Quality,
    pub contrast: Contrast,
}

impl RenderRequest {
    pub fn new(viewport_width: u32, viewport_height: u32) -> Self {
        Self {
            viewport_width,
            viewport_height,
            fit: FitMode::Page,
            quality: Quality::Normal,
            contrast: Contrast::Normal,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    Jpeg,
    Png,
}

/// Detect the format from magic bytes (never trust the file name).
pub fn sniff(bytes: &[u8]) -> Option<ImageKind> {
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(ImageKind::Jpeg)
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageKind::Png)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageHeader {
    pub kind: ImageKind,
    pub width: u32,
    pub height: u32,
    /// Progressive JPEG / interlaced PNG: needs a full-size intermediate.
    pub needs_full_frame: bool,
}

#[derive(Debug)]
pub enum RenderError {
    UnsupportedFormat(String),
    /// Exceeds a hard limit or the memory budget. Retry at lower quality may work.
    TooLarge(String),
    Corrupt(String),
    Cancelled,
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFormat(m) => write!(f, "unsupported image: {m}"),
            Self::TooLarge(m) => write!(f, "page needs too much memory: {m}"),
            Self::Corrupt(m) => write!(f, "damaged image: {m}"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::error::Error for RenderError {}

impl From<BudgetExceeded> for RenderError {
    fn from(e: BudgetExceeded) -> Self {
        Self::TooLarge(e.to_string())
    }
}

/// A prepared page. Holds its budget reservation until dropped.
pub struct GrayImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    _reservation: Option<Reservation>,
}

impl fmt::Debug for GrayImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GrayImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl GrayImage {
    /// Wrap pixels that were not produced under a budget (tests, IPC copies).
    pub fn from_pixels(width: u32, height: u32, pixels: Vec<u8>) -> Option<Self> {
        (required_bytes(width, height, 1)? == pixels.len()).then_some(Self {
            width,
            height,
            pixels,
            _reservation: None,
        })
    }

    /// Attach a budget reservation covering this buffer.
    pub fn with_reservation(mut self, r: Reservation) -> Self {
        self._reservation = Some(r);
        self
    }
}

/// Read only the image header.
pub fn probe(bytes: &[u8]) -> Result<ImageHeader, RenderError> {
    match sniff(bytes) {
        Some(ImageKind::Jpeg) => {
            let mut d = jpeg_decoder::Decoder::new(bytes);
            d.read_info()
                .map_err(|e| RenderError::Corrupt(e.to_string()))?;
            let info = d
                .info()
                .ok_or_else(|| RenderError::Corrupt("missing JPEG header".into()))?;
            Ok(ImageHeader {
                kind: ImageKind::Jpeg,
                width: info.width as u32,
                height: info.height as u32,
                needs_full_frame: info.coding_process != jpeg_decoder::CodingProcess::DctSequential,
            })
        }
        Some(ImageKind::Png) => {
            let mut d = png::Decoder::new(Cursor::new(bytes));
            let info = d.read_header_info().map_err(png_err)?;
            Ok(ImageHeader {
                kind: ImageKind::Png,
                width: info.width,
                height: info.height,
                needs_full_frame: info.interlaced,
            })
        }
        None => Err(RenderError::UnsupportedFormat(
            "not a JPEG or PNG file".into(),
        )),
    }
}

fn validate_header(h: &ImageHeader, limits: &Limits) -> Result<(), RenderError> {
    if h.width == 0 || h.height == 0 {
        return Err(RenderError::Corrupt("zero-sized image".into()));
    }
    if h.width > limits.max_source_dimension || h.height > limits.max_source_dimension {
        return Err(RenderError::TooLarge(format!(
            "{}×{} exceeds max dimension {}",
            h.width, h.height, limits.max_source_dimension
        )));
    }
    let pixels = h.width as u64 * h.height as u64;
    if pixels > limits.max_source_pixels {
        return Err(RenderError::TooLarge(format!(
            "{}×{} = {pixels} pixels exceeds {}",
            h.width, h.height, limits.max_source_pixels
        )));
    }
    Ok(())
}

/// Output size for a source, never exceeding the source in either axis.
pub fn output_size(src_w: u32, src_h: u32, req: &RenderRequest, limits: &Limits) -> (u32, u32) {
    let (vw, vh) = (
        req.viewport_width.max(1) as f64,
        req.viewport_height.max(1) as f64,
    );
    let (sw, sh) = (src_w.max(1) as f64, src_h.max(1) as f64);
    let mut s = match req.fit {
        FitMode::Page => (vw / sw).min(vh / sh),
        FitMode::Width => vw / sw,
    };
    if req.quality == Quality::LowMemory {
        s *= 0.5;
    }
    s = s.min(1.0);
    // Respect the output pixel cap (e.g. extremely tall strips in fit-width).
    let max_px = limits.max_output_pixels.max(1) as f64;
    let round: fn(f64) -> f64 = if sw * s * sh * s > max_px {
        s = (max_px / (sw * sh)).sqrt();
        f64::floor
    } else {
        f64::round
    };
    let w = (round(sw * s) as u32).clamp(1, src_w.max(1));
    let h = (round(sh * s) as u32).clamp(1, src_h.max(1));
    (w, h)
}

/// Decode `bytes` into a display-sized grayscale page within `budget`.
///
/// `cancelled` is polled between rows/stages; returning `true` aborts with
/// [`RenderError::Cancelled`] and releases all reservations.
pub fn render_gray(
    bytes: &[u8],
    req: &RenderRequest,
    limits: &Limits,
    budget: &MemoryBudget,
    cancelled: &dyn Fn() -> bool,
) -> Result<GrayImage, RenderError> {
    let header = probe(bytes)?;
    validate_header(&header, limits)?;
    let (out_w, out_h) = output_size(header.width, header.height, req, limits);
    let out_bytes = required_bytes(out_w, out_h, 1)
        .ok_or_else(|| RenderError::TooLarge("output size overflow".into()))?;
    // Output buffer is reserved separately and travels with the GrayImage.
    let out_reservation = budget.reserve(out_bytes, "page buffer")?;
    if cancelled() {
        return Err(RenderError::Cancelled);
    }
    let mut img = match header.kind {
        ImageKind::Jpeg => decode_jpeg(bytes, &header, out_w, out_h, budget, cancelled)?,
        ImageKind::Png => decode_png(bytes, &header, out_w, out_h, budget, cancelled)?,
    };
    apply_contrast(&mut img.pixels, req.contrast);
    Ok(img.with_reservation(out_reservation))
}

fn apply_contrast(px: &mut [u8], c: Contrast) {
    if c == Contrast::Normal {
        return;
    }
    let mut lut = [0u8; 256];
    for (i, v) in lut.iter_mut().enumerate() {
        *v = (255.0 * (i as f64 / 255.0).powf(1.6)).round() as u8;
    }
    for p in px {
        *p = lut[*p as usize];
    }
}

#[inline]
fn luma(r: u8, g: u8, b: u8) -> u8 {
    ((77 * r as u32 + 150 * g as u32 + 29 * b as u32 + 128) >> 8) as u8
}

#[inline]
fn over_white(v: u8, a: u8) -> u8 {
    ((v as u32 * a as u32 + 255 * (255 - a as u32) + 127) / 255) as u8
}

fn decode_jpeg(
    bytes: &[u8],
    header: &ImageHeader,
    out_w: u32,
    out_h: u32,
    budget: &MemoryBudget,
    cancelled: &dyn Fn() -> bool,
) -> Result<GrayImage, RenderError> {
    use jpeg_decoder::PixelFormat;
    let mut d = jpeg_decoder::Decoder::new(bytes);
    d.read_info()
        .map_err(|e| RenderError::Corrupt(e.to_string()))?;
    let info = d.info().expect("read_info succeeded");
    let req_w = u16::try_from(out_w).unwrap_or(u16::MAX);
    let req_h = u16::try_from(out_h).unwrap_or(u16::MAX);
    let (sw, sh) = d
        .scale(req_w, req_h)
        .map_err(|e| RenderError::Corrupt(e.to_string()))?;
    let (sw, sh) = (sw as u32, sh as u32);
    let bpp = info.pixel_format.pixel_bytes() as u32;
    // Planning estimate: decoder planes + returned Vec at scaled size, plus
    // full-resolution i16 coefficients for progressive images.
    let scaled = required_bytes(sw, sh, bpp)
        .and_then(|b| b.checked_mul(2))
        .ok_or_else(|| RenderError::TooLarge("scaled size overflow".into()))?;
    let coeffs = if header.needs_full_frame {
        required_bytes(header.width, header.height, bpp * 2)
            .ok_or_else(|| RenderError::TooLarge("coefficient size overflow".into()))?
    } else {
        0
    };
    let scratch = budget.reserve(scaled.saturating_add(coeffs), "JPEG decode")?;
    d.set_max_decoding_buffer_size(scratch.bytes().max(1));
    let data = d.decode().map_err(|e| match e {
        jpeg_decoder::Error::Unsupported(u) => RenderError::UnsupportedFormat(format!("{u:?}")),
        other => RenderError::Corrupt(other.to_string()),
    })?;
    if cancelled() {
        return Err(RenderError::Cancelled);
    }
    let expected = required_bytes(sw, sh, bpp).unwrap_or(usize::MAX);
    if data.len() < expected {
        return Err(RenderError::Corrupt("short JPEG output".into()));
    }
    let (ow, oh) = (out_w.min(sw), out_h.min(sh));
    let mut ds = AreaDownscaler::new(sw, sh, ow, oh);
    let mut row = vec![0u8; sw as usize];
    let stride = (sw * bpp) as usize;
    for y in 0..sh as usize {
        let src = &data[y * stride..(y + 1) * stride];
        match info.pixel_format {
            PixelFormat::L8 => row.copy_from_slice(src),
            PixelFormat::L16 => {
                for (o, px) in row.iter_mut().zip(src.chunks_exact(2)) {
                    *o = px[0];
                }
            }
            PixelFormat::RGB24 => {
                for (o, px) in row.iter_mut().zip(src.chunks_exact(3)) {
                    *o = luma(px[0], px[1], px[2]);
                }
            }
            PixelFormat::CMYK32 => {
                for (o, px) in row.iter_mut().zip(src.chunks_exact(4)) {
                    // jpeg-decoder returns inverted (Adobe) CMYK; approximate.
                    let k = px[3] as u32;
                    let c = |v: u8| ((v as u32 * k) / 255) as u8;
                    *o = luma(c(px[0]), c(px[1]), c(px[2]));
                }
            }
        }
        ds.push_row(&row);
        if y.is_multiple_of(64) && cancelled() {
            return Err(RenderError::Cancelled);
        }
    }
    drop(data);
    drop(scratch);
    Ok(ds.finish())
}

fn png_err(e: png::DecodingError) -> RenderError {
    match e {
        png::DecodingError::LimitsExceeded => RenderError::TooLarge("PNG decoder limit".into()),
        png::DecodingError::Parameter(p) => RenderError::UnsupportedFormat(p.to_string()),
        other => RenderError::Corrupt(other.to_string()),
    }
}

fn decode_png(
    bytes: &[u8],
    header: &ImageHeader,
    out_w: u32,
    out_h: u32,
    budget: &MemoryBudget,
    cancelled: &dyn Fn() -> bool,
) -> Result<GrayImage, RenderError> {
    let (w, h) = (header.width, header.height);
    // Row scratch: raw + previous row inside the decoder, our converted row,
    // and the downscaler's column map/accumulator.
    let row_scratch = required_bytes(w, 1, 8 * 2 + 1 + 4)
        .ok_or_else(|| RenderError::TooLarge("row size overflow".into()))?;
    let full_frame = if header.needs_full_frame {
        required_bytes(w, h, 4)
            .ok_or_else(|| RenderError::TooLarge("frame size overflow".into()))?
    } else {
        0
    };
    let scratch = budget.reserve(row_scratch.saturating_add(full_frame), "PNG decode")?;
    let mut d = png::Decoder::new_with_limits(
        Cursor::new(bytes),
        png::Limits {
            bytes: scratch.bytes().max(64 * 1024),
        },
    );
    d.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = d.read_info().map_err(png_err)?;
    let (color, _) = reader.output_color_type();
    let channels = color.samples();
    let to_gray = |src: &[u8], dst: &mut [u8]| match channels {
        1 => dst.copy_from_slice(&src[..dst.len()]),
        2 => {
            for (o, p) in dst.iter_mut().zip(src.chunks_exact(2)) {
                *o = over_white(p[0], p[1]);
            }
        }
        3 => {
            for (o, p) in dst.iter_mut().zip(src.chunks_exact(3)) {
                *o = luma(p[0], p[1], p[2]);
            }
        }
        _ => {
            for (o, p) in dst.iter_mut().zip(src.chunks_exact(4)) {
                *o = over_white(luma(p[0], p[1], p[2]), p[3]);
            }
        }
    };
    let mut ds = AreaDownscaler::new(w, h, out_w, out_h);
    let mut gray = vec![0u8; w as usize];
    if !header.needs_full_frame {
        let mut y = 0u32;
        while let Some(row) = reader.next_row().map_err(png_err)? {
            let data = row.data();
            if data.len() < w as usize * channels {
                return Err(RenderError::Corrupt("short PNG row".into()));
            }
            to_gray(data, &mut gray);
            ds.push_row(&gray);
            y += 1;
            if y.is_multiple_of(64) && cancelled() {
                return Err(RenderError::Cancelled);
            }
        }
        if y != h {
            return Err(RenderError::Corrupt(format!(
                "PNG ended after {y} of {h} rows"
            )));
        }
    } else {
        let size = reader
            .output_buffer_size()
            .ok_or_else(|| RenderError::TooLarge("PNG frame size".into()))?;
        if size > full_frame {
            return Err(RenderError::TooLarge("interlaced PNG frame".into()));
        }
        let mut frame = vec![0u8; size];
        let out = reader.next_frame(&mut frame).map_err(png_err)?;
        if cancelled() {
            return Err(RenderError::Cancelled);
        }
        let stride = out.line_size;
        for y in 0..h as usize {
            to_gray(&frame[y * stride..(y + 1) * stride], &mut gray);
            ds.push_row(&gray);
        }
    }
    drop(scratch);
    Ok(ds.finish())
}

#[cfg(test)]
mod tests;
