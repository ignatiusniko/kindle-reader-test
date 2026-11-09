//! Minimal `/proc/meminfo` reader.

use std::path::Path;

/// Values from `/proc/meminfo`, in bytes. `None` = not reported by kernel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemInfo {
    pub total: Option<usize>,
    pub free: Option<usize>,
    pub available: Option<usize>,
    pub cached: Option<usize>,
    pub swap_free: Option<usize>,
}

impl MemInfo {
    pub fn read() -> std::io::Result<Self> {
        Self::read_from(Path::new("/proc/meminfo"))
    }

    pub fn read_from(path: &Path) -> std::io::Result<Self> {
        Ok(Self::parse(&std::fs::read_to_string(path)?))
    }

    pub fn parse(text: &str) -> Self {
        let mut info = Self::default();
        for line in text.lines() {
            let mut parts = line.split_whitespace();
            let (Some(key), Some(value)) = (parts.next(), parts.next()) else {
                continue;
            };
            let Ok(kib) = value.parse::<usize>() else {
                continue;
            };
            let bytes = Some(kib.saturating_mul(1024));
            match key {
                "MemTotal:" => info.total = bytes,
                "MemFree:" => info.free = bytes,
                "MemAvailable:" => info.available = bytes,
                "Cached:" => info.cached = bytes,
                "SwapFree:" => info.swap_free = bytes,
                _ => {}
            }
        }
        info
    }

    /// `MemAvailable`, or a pessimistic `MemFree` fallback on old kernels.
    /// Deliberately does *not* add page cache to the fallback.
    pub fn effective_available(&self) -> Option<usize> {
        self.available.or(self.free)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typical_output() {
        let m = MemInfo::parse(
            "MemTotal:         503000 kB\nMemFree:           20000 kB\nMemAvailable:     180000 kB\nCached:  1 kB\n",
        );
        assert_eq!(m.total, Some(503000 * 1024));
        assert_eq!(m.effective_available(), Some(180000 * 1024));
    }

    #[test]
    fn falls_back_to_free_without_mem_available() {
        let m = MemInfo::parse("MemTotal: 10 kB\nMemFree: 4 kB\n");
        assert_eq!(m.effective_available(), Some(4096));
        assert_eq!(MemInfo::parse("garbage").effective_available(), None);
    }
}
