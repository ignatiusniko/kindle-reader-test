//! Tap zones, reading direction, fit-width segments and the duplicate-tap gate.

use std::time::{Duration, Instant};

use persistence::Direction;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavAction {
    Next,
    Previous,
    ShowOptions,
}

/// Left 30% / centre 40% / right 30% of the screen width.
pub fn zone_for(x: f32, width: f32) -> Zone {
    if width <= 0.0 {
        return Zone::Center;
    }
    let f = x / width;
    if f < 0.3 {
        Zone::Left
    } else if f > 0.7 {
        Zone::Right
    } else {
        Zone::Center
    }
}

/// Physical zone → logical action. With RTL manga, tapping the **left**
/// edge advances (the next page is "to the left").
pub fn action_for(zone: Zone, direction: Direction) -> NavAction {
    match (zone, direction) {
        (Zone::Center, _) => NavAction::ShowOptions,
        (Zone::Right, Direction::LeftToRight) | (Zone::Left, Direction::RightToLeft) => {
            NavAction::Next
        }
        _ => NavAction::Previous,
    }
}

/// Fraction of the screen kept visible when stepping to the next segment.
pub const SEGMENT_OVERLAP: f32 = 0.1;

/// Vertical offsets (in display pixels) for reading a tall fit-width page
/// one screen at a time. Always returns at least `[0.0]`.
pub fn segment_offsets(display_height: f32, viewport_height: f32) -> Vec<f32> {
    if viewport_height <= 0.0 || display_height <= viewport_height * 1.02 {
        return vec![0.0];
    }
    let max = display_height - viewport_height;
    let step = viewport_height * (1.0 - SEGMENT_OVERLAP);
    let mut v = Vec::new();
    let mut y = 0.0;
    while y < max {
        v.push(y);
        y += step;
    }
    v.push(max);
    v
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Same image, scroll to this segment.
    Segment(u32),
    /// Load another page.
    Page(u32),
    AtEnd,
    AtStart,
}

/// Page + segment position within a book.
#[derive(Debug, Clone)]
pub struct Navigator {
    page_count: u32,
    page: u32,
    segment: u32,
    segments: u32,
    /// After "previous page" in fit-width, start at the bottom once loaded.
    start_at_end: bool,
}

impl Navigator {
    pub fn new(page_count: u32, start_page: u32) -> Self {
        Self {
            page_count: page_count.max(1),
            page: start_page.min(page_count.saturating_sub(1)),
            segment: 0,
            segments: 1,
            start_at_end: false,
        }
    }

    pub fn page(&self) -> u32 {
        self.page
    }
    pub fn page_count(&self) -> u32 {
        self.page_count
    }
    pub fn segment(&self) -> u32 {
        self.segment
    }
    pub fn segments(&self) -> u32 {
        self.segments
    }

    /// Called when a page image arrives and its segment count is known.
    pub fn set_segments(&mut self, n: u32) {
        self.segments = n.max(1);
        self.segment = if self.start_at_end {
            self.segments - 1
        } else {
            0
        };
        self.start_at_end = false;
    }

    pub fn forward(&mut self) -> Step {
        if self.segment + 1 < self.segments {
            self.segment += 1;
            Step::Segment(self.segment)
        } else if self.page + 1 < self.page_count {
            self.page += 1;
            self.segment = 0;
            self.segments = 1;
            self.start_at_end = false;
            Step::Page(self.page)
        } else {
            Step::AtEnd
        }
    }

    pub fn backward(&mut self) -> Step {
        if self.segment > 0 {
            self.segment -= 1;
            Step::Segment(self.segment)
        } else if self.page > 0 {
            self.page -= 1;
            self.segment = 0;
            self.segments = 1;
            self.start_at_end = true;
            Step::Page(self.page)
        } else {
            Step::AtStart
        }
    }

    pub fn jump(&mut self, page: u32) -> Step {
        let page = page.min(self.page_count - 1);
        self.page = page;
        self.segment = 0;
        self.segments = 1;
        self.start_at_end = false;
        Step::Page(page)
    }
}

/// Drops taps that arrive while a page is loading or too soon after the
/// previous accepted tap (E-Ink refresh makes users double-tap).
#[derive(Debug)]
pub struct TapGate {
    min_interval: Duration,
    last: Option<Instant>,
}

impl TapGate {
    pub fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            last: None,
        }
    }

    pub fn accept(&mut self, now: Instant, busy: bool) -> bool {
        if busy {
            return false;
        }
        if let Some(last) = self.last
            && now.duration_since(last) < self.min_interval
        {
            return false;
        }
        self.last = Some(now);
        true
    }
}

impl Default for TapGate {
    fn default() -> Self {
        Self::new(Duration::from_millis(300))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zones_and_direction() {
        assert_eq!(zone_for(10.0, 1072.0), Zone::Left);
        assert_eq!(zone_for(536.0, 1072.0), Zone::Center);
        assert_eq!(zone_for(1000.0, 1072.0), Zone::Right);
        assert_eq!(
            action_for(Zone::Right, Direction::LeftToRight),
            NavAction::Next
        );
        assert_eq!(
            action_for(Zone::Left, Direction::LeftToRight),
            NavAction::Previous
        );
        assert_eq!(
            action_for(Zone::Left, Direction::RightToLeft),
            NavAction::Next
        );
        assert_eq!(
            action_for(Zone::Right, Direction::RightToLeft),
            NavAction::Previous
        );
        assert_eq!(
            action_for(Zone::Center, Direction::RightToLeft),
            NavAction::ShowOptions
        );
    }

    #[test]
    fn segments() {
        assert_eq!(segment_offsets(1000.0, 1448.0), vec![0.0]);
        let s = segment_offsets(3000.0, 1000.0);
        assert_eq!(s.first(), Some(&0.0));
        assert_eq!(s.last(), Some(&2000.0));
        assert!(s.windows(2).all(|w| w[1] - w[0] <= 900.0 + 0.01));
    }

    #[test]
    fn navigator_pages_and_segments() {
        let mut n = Navigator::new(3, 0);
        n.set_segments(2);
        assert_eq!(n.forward(), Step::Segment(1));
        assert_eq!(n.forward(), Step::Page(1));
        n.set_segments(1);
        assert_eq!(n.backward(), Step::Page(0));
        n.set_segments(2); // start at the bottom when going back
        assert_eq!(n.segment(), 1);
        assert_eq!(n.backward(), Step::Segment(0));
        assert_eq!(n.backward(), Step::AtStart);
        assert_eq!(n.jump(99), Step::Page(2));
        n.set_segments(1);
        assert_eq!(n.forward(), Step::AtEnd);
        assert_eq!(Navigator::new(5, 50).page(), 4);
    }

    #[test]
    fn tap_gate() {
        let mut g = TapGate::new(Duration::from_millis(300));
        let t0 = Instant::now();
        assert!(g.accept(t0, false));
        assert!(!g.accept(t0 + Duration::from_millis(100), false));
        assert!(!g.accept(t0 + Duration::from_millis(400), true));
        assert!(g.accept(t0 + Duration::from_millis(400), false));
    }
}
