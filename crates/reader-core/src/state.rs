//! Explicit reader state machine (spec §23.3). Invalid transitions are
//! rejected instead of silently leaving the UI in a frozen state.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReaderState {
    Idle,
    OpeningArchive,
    DecodingPage,
    Reading,
    RecoverableError,
    Closing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReaderEvent {
    Open,
    /// Archive opened and indexed (metadata only).
    Indexed,
    RequestPage,
    /// Prepared page handed to the display.
    Presented,
    Fail,
    Close,
    Closed,
}

impl ReaderState {
    pub fn on(self, ev: ReaderEvent) -> Option<Self> {
        use ReaderEvent as E;
        use ReaderState as S;
        Some(match (self, ev) {
            (S::Idle, E::Open) => S::OpeningArchive,
            (S::OpeningArchive, E::Indexed) => S::DecodingPage,
            // A new request replaces (cancels) any in-flight decode.
            (S::DecodingPage | S::Reading | S::RecoverableError, E::RequestPage) => S::DecodingPage,
            (S::DecodingPage, E::Presented) => S::Reading,
            (S::OpeningArchive | S::DecodingPage | S::Reading, E::Fail) => S::RecoverableError,
            (S::Idle, E::Close) => S::Idle,
            (_, E::Close) => S::Closing,
            (S::Closing, E::Closed) => S::Idle,
            _ => return None,
        })
    }

    pub fn is_busy(self) -> bool {
        matches!(self, Self::OpeningArchive | Self::DecodingPage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ReaderEvent as E;
    use ReaderState as S;

    #[test]
    fn happy_path() {
        let mut s = S::Idle;
        for ev in [
            E::Open,
            E::Indexed,
            E::Presented,
            E::RequestPage,
            E::Presented,
            E::Close,
            E::Closed,
        ] {
            s = s.on(ev).unwrap_or_else(|| panic!("{s:?} + {ev:?}"));
        }
        assert_eq!(s, S::Idle);
    }

    #[test]
    fn errors_are_recoverable_and_invalid_moves_rejected() {
        assert_eq!(S::DecodingPage.on(E::Fail), Some(S::RecoverableError));
        assert_eq!(
            S::RecoverableError.on(E::RequestPage),
            Some(S::DecodingPage)
        );
        assert_eq!(S::RecoverableError.on(E::Close), Some(S::Closing));
        assert_eq!(S::Idle.on(E::Presented), None);
        assert_eq!(S::Reading.on(E::Presented), None);
        assert!(S::DecodingPage.is_busy());
        assert!(!S::Reading.is_busy());
    }
}
