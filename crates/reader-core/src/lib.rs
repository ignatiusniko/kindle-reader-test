//! UI-agnostic reader core (spec §4, §23, §24).
//!
//! The Slint app is a thin shell around these pieces:
//!
//! * [`catalog`] — find CBZ files in the book folder (no page decoding).
//! * [`navigation`] — tap zones, reading direction, fit-width segments,
//!   duplicate-tap gate.
//! * [`state`] — explicit reader state machine.
//! * [`render`] — the page request/result contract and the in-process and
//!   out-of-process renderers.
//! * [`scheduler`] — one background worker, queue capacity 1, latest
//!   request wins, stale results are dropped by generation id.
//! * [`controller`] — ties it together into a testable, event-driven
//!   controller that the UI calls and that emits [`controller::Output`]s.

pub mod catalog;
pub mod controller;
pub mod navigation;
pub mod protocol;
pub mod render;
pub mod scheduler;
pub mod state;

pub use image_pipeline::{Contrast, FitMode, GrayImage, Quality, RenderRequest};
pub use persistence::{ContrastPref, Direction, Fit, ReadingPrefs};

pub fn fit_mode(f: Fit) -> FitMode {
    match f {
        Fit::Page => FitMode::Page,
        Fit::Width => FitMode::Width,
    }
}

pub fn contrast(c: ContrastPref) -> Contrast {
    match c {
        ContrastPref::Normal => Contrast::Normal,
        ContrastPref::Darker => Contrast::Darker,
    }
}
