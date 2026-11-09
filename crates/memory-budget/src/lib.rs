//! Memory policy for the reader (spec §5 "Memory policy", §24.4).
//!
//! * [`required_bytes`] — every `width × height × bpp` goes through checked
//!   arithmetic.
//! * [`Limits`] — hard caps for untrusted input (ZIP entries, image pixels…).
//! * [`MemoryBudget`] — a reservation counter: reserve *before* allocating,
//!   release automatically when the [`Reservation`] is dropped (on success,
//!   error or cancellation alike).
//!
//! The reservation count is a **planning guard**, not a measurement: native
//! allocations, fragmentation and Slint copies are not visible to it. Peak
//! RSS/PSS must still be measured on the device (see `diagnostic-kit`).

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

pub mod meminfo;

pub const KIB: usize = 1024;
pub const MIB: usize = 1024 * 1024;

/// Bytes needed for a `width × height` raster with `bytes_per_pixel`.
///
/// Returns `None` on overflow or if the result does not fit in `usize`
/// (relevant on 32-bit ARM, where `usize` is 32 bits).
pub fn required_bytes(width: u32, height: u32, bytes_per_pixel: u32) -> Option<usize> {
    let pixels = (width as u64).checked_mul(height as u64)?;
    let bytes = pixels.checked_mul(bytes_per_pixel as u64)?;
    usize::try_from(bytes).ok()
}

/// Hard limits applied to untrusted books before any large allocation.
///
/// Defaults are conservative *starting points* for a 512 MiB-class device and
/// must be recalibrated from on-device measurements (spec §12 M2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Maximum number of entries in a ZIP central directory.
    pub max_archive_entries: usize,
    /// Maximum number of readable pages in one book.
    pub max_pages: usize,
    /// Maximum *declared and actual* uncompressed size of one page entry.
    pub uncompressed_entry_limit: usize,
    /// Maximum uncompressed/compressed ratio (decompression-bomb guard).
    pub max_compression_ratio: u64,
    /// Ratio guard only applies to entries larger than this.
    pub ratio_check_min_bytes: usize,
    /// Maximum source image pixels (width × height) we will even attempt.
    pub max_source_pixels: u64,
    /// Maximum source width or height.
    pub max_source_dimension: u32,
    /// Maximum output pixels for one prepared page.
    pub max_output_pixels: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_archive_entries: 20_000,
            max_pages: 10_000,
            uncompressed_entry_limit: 32 * MIB,
            max_compression_ratio: 100,
            ratio_check_min_bytes: MIB,
            max_source_pixels: 60_000_000,
            max_source_dimension: 30_000,
            // Fit-width on a 1072-wide screen allows tall strips up to ~4 screens.
            max_output_pixels: 1072 * 1448 * 4,
        }
    }
}

/// Budget split derived from available memory (spec §5, §24.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetPlan {
    /// Total new memory the app may use: `min(64 MiB, 35% of MemAvailable)`.
    pub process_budget: usize,
    /// Cap for decoder scratch + compressed input of a single page.
    pub decode_budget: usize,
    /// Cap for prepared page buffers (current page + optional cache).
    pub page_buffer_budget: usize,
    /// Whether prefetching one page is allowed. Off by default on KT5.
    pub allow_prefetch: bool,
}

impl BudgetPlan {
    pub const HARD_CAP: usize = 64 * MIB;
    /// Below this the plan still works but refuses most large pages.
    pub const FLOOR: usize = 16 * MIB;

    /// Derive the initial plan from `MemAvailable` in bytes.
    pub fn from_available(mem_available: usize) -> Self {
        let share = mem_available / 100 * 35;
        let process_budget = share.min(Self::HARD_CAP).max(Self::FLOOR.min(share));
        // ~60% decode scratch, ~30% page buffers, remainder UI/IPC slack.
        let decode_budget = process_budget / 10 * 6;
        let page_buffer_budget = process_budget / 10 * 3;
        Self {
            process_budget,
            decode_budget,
            page_buffer_budget,
            allow_prefetch: false,
        }
    }

    /// Plan used when `/proc/meminfo` is unavailable (host tests, odd kernels).
    pub fn conservative_default() -> Self {
        Self::from_available(128 * MIB)
    }
}

/// Error returned when a reservation would exceed the budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetExceeded {
    pub requested: usize,
    pub in_use: usize,
    pub limit: usize,
    pub purpose: &'static str,
}

impl fmt::Display for BudgetExceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: needs {} KiB but only {} of {} KiB budget is free",
            self.purpose,
            self.requested / KIB,
            self.limit.saturating_sub(self.in_use) / KIB,
            self.limit / KIB
        )
    }
}

impl std::error::Error for BudgetExceeded {}

#[derive(Debug)]
struct Inner {
    limit: usize,
    in_use: AtomicUsize,
    peak: AtomicUsize,
}

/// Thread-safe reservation counter. Cheap to clone (shared state).
#[derive(Debug, Clone)]
pub struct MemoryBudget {
    inner: Arc<Inner>,
}

impl MemoryBudget {
    pub fn new(limit: usize) -> Self {
        Self {
            inner: Arc::new(Inner {
                limit,
                in_use: AtomicUsize::new(0),
                peak: AtomicUsize::new(0),
            }),
        }
    }

    pub fn limit(&self) -> usize {
        self.inner.limit
    }

    pub fn in_use(&self) -> usize {
        self.inner.in_use.load(Ordering::Acquire)
    }

    /// Highest reserved total seen so far.
    pub fn peak(&self) -> usize {
        self.inner.peak.load(Ordering::Acquire)
    }

    pub fn available(&self) -> usize {
        self.limit().saturating_sub(self.in_use())
    }

    /// Reserve `bytes`; fails without side effects if it would exceed the limit.
    pub fn reserve(
        &self,
        bytes: usize,
        purpose: &'static str,
    ) -> Result<Reservation, BudgetExceeded> {
        let mut current = self.inner.in_use.load(Ordering::Acquire);
        loop {
            let next = current
                .checked_add(bytes)
                .filter(|n| *n <= self.inner.limit);
            let Some(next) = next else {
                return Err(BudgetExceeded {
                    requested: bytes,
                    in_use: current,
                    limit: self.inner.limit,
                    purpose,
                });
            };
            match self.inner.in_use.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.inner.peak.fetch_max(next, Ordering::AcqRel);
                    return Ok(Reservation {
                        budget: self.clone(),
                        bytes,
                    });
                }
                Err(actual) => current = actual,
            }
        }
    }
}

/// RAII guard for reserved bytes. Dropping it releases the reservation.
#[derive(Debug)]
pub struct Reservation {
    budget: MemoryBudget,
    bytes: usize,
}

impl Reservation {
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Shrink the reservation once the real size is known.
    pub fn shrink_to(&mut self, bytes: usize) {
        if bytes < self.bytes {
            let delta = self.bytes - bytes;
            self.budget.inner.in_use.fetch_sub(delta, Ordering::AcqRel);
            self.bytes = bytes;
        }
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget
            .inner
            .in_use
            .fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_bytes_handles_extremes() {
        assert_eq!(required_bytes(1072, 1448, 1), Some(1_552_256));
        assert_eq!(required_bytes(1072, 1448, 3), Some(4_656_768));
        assert_eq!(required_bytes(1072, 1448, 4), Some(6_209_024));
        assert_eq!(required_bytes(0, u32::MAX, 4), Some(0));
        // u32::MAX² × 4 overflows u64.
        assert_eq!(required_bytes(u32::MAX, u32::MAX, 4), None);
        // u32::MAX² fits u64 but not a 32-bit usize; on 64-bit it fits.
        let r = required_bytes(u32::MAX, u32::MAX, 1);
        if usize::BITS == 32 {
            assert_eq!(r, None);
        } else {
            assert!(r.is_some());
        }
    }

    #[test]
    fn plan_matches_spec_formula() {
        // 35% of 100 MiB = 35 MiB < 64 MiB cap.
        let p = BudgetPlan::from_available(100 * MIB);
        assert_eq!(p.process_budget, 100 * MIB / 100 * 35);
        assert!(!p.allow_prefetch);
        // Plenty of RAM: capped at 64 MiB.
        assert_eq!(
            BudgetPlan::from_available(400 * MIB).process_budget,
            64 * MIB
        );
        assert!(p.decode_budget + p.page_buffer_budget <= p.process_budget);
    }

    #[test]
    fn reservations_are_released_on_drop() {
        let b = MemoryBudget::new(10 * MIB);
        let r1 = b.reserve(6 * MIB, "a").unwrap();
        let err = b.reserve(5 * MIB, "b").unwrap_err();
        assert_eq!(err.requested, 5 * MIB);
        assert_eq!(b.in_use(), 6 * MIB);
        drop(r1);
        assert_eq!(b.in_use(), 0);
        let mut r2 = b.reserve(10 * MIB, "c").unwrap();
        r2.shrink_to(MIB);
        assert_eq!(b.in_use(), MIB);
        assert_eq!(b.peak(), 10 * MIB);
    }

    #[test]
    fn reserve_overflow_is_refused() {
        let b = MemoryBudget::new(usize::MAX);
        let _r = b.reserve(usize::MAX - 1, "a").unwrap();
        assert!(b.reserve(2, "b").is_err());
    }
}
