//! Streaming box-filter downscaler for Gray8 rows.

use crate::GrayImage;

/// Feeds source rows top to bottom; memory is `O(dst_w + src_w)` plus output.
pub struct AreaDownscaler {
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
    /// Destination column for each source column.
    col_map: Vec<u32>,
    /// Number of source columns mapping to each destination column.
    col_count: Vec<u32>,
    acc: Vec<u64>,
    band_rows: u32,
    band: u32,
    next_src_row: u32,
    out: Vec<u8>,
}

impl AreaDownscaler {
    /// `dst` must not exceed `src` in either axis (callers clamp).
    pub fn new(src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Self {
        let dst_w = dst_w.clamp(1, src_w.max(1));
        let dst_h = dst_h.clamp(1, src_h.max(1));
        let mut col_map = Vec::with_capacity(src_w as usize);
        let mut col_count = vec![0u32; dst_w as usize];
        for x in 0..src_w {
            let dx = ((x as u64 * dst_w as u64) / src_w as u64) as u32;
            col_map.push(dx);
            col_count[dx as usize] += 1;
        }
        Self {
            src_w,
            src_h,
            dst_w,
            dst_h,
            col_map,
            col_count,
            acc: vec![0; dst_w as usize],
            band_rows: 0,
            band: 0,
            next_src_row: 0,
            out: vec![0; dst_w as usize * dst_h as usize],
        }
    }

    fn band_of(&self, y: u32) -> u32 {
        ((y as u64 * self.dst_h as u64) / self.src_h as u64) as u32
    }

    fn flush_band(&mut self) {
        if self.band_rows == 0 {
            return;
        }
        let base = self.band as usize * self.dst_w as usize;
        for (dx, sum) in self.acc.iter_mut().enumerate() {
            let n = self.col_count[dx] as u64 * self.band_rows as u64;
            self.out[base + dx] = ((*sum + n / 2) / n.max(1)) as u8;
            *sum = 0;
        }
        self.band_rows = 0;
    }

    /// Push the next source row (`src_w` gray pixels). Extra rows are ignored.
    pub fn push_row(&mut self, row: &[u8]) {
        if self.next_src_row >= self.src_h {
            return;
        }
        let band = self.band_of(self.next_src_row);
        if band != self.band {
            self.flush_band();
            self.band = band;
        }
        for (x, &v) in row.iter().take(self.src_w as usize).enumerate() {
            self.acc[self.col_map[x] as usize] += v as u64;
        }
        self.band_rows += 1;
        self.next_src_row += 1;
    }

    pub fn finish(mut self) -> GrayImage {
        self.flush_band();
        GrayImage::from_pixels(self.dst_w, self.dst_h, self.out).expect("sized correctly")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn averages_blocks() {
        let mut d = AreaDownscaler::new(4, 2, 2, 1);
        d.push_row(&[0, 100, 200, 200]);
        d.push_row(&[100, 0, 0, 0]);
        let img = d.finish();
        assert_eq!((img.width, img.height), (2, 1));
        assert_eq!(img.pixels, vec![50, 100]);
    }

    #[test]
    fn identity_and_uneven() {
        let mut d = AreaDownscaler::new(3, 3, 3, 3);
        for r in 0..3u8 {
            d.push_row(&[r, r + 10, r + 20]);
        }
        assert_eq!(d.finish().pixels, vec![0, 10, 20, 1, 11, 21, 2, 12, 22]);
        let mut d = AreaDownscaler::new(7, 5, 3, 2);
        for _ in 0..5 {
            d.push_row(&[255; 7]);
        }
        assert!(d.finish().pixels.iter().all(|&p| p == 255));
    }
}
