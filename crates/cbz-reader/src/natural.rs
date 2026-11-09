//! Natural ("human") filename ordering: `2.jpg` < `10.jpg`, case-insensitive.

use std::cmp::Ordering;

#[derive(Debug, PartialEq, Eq)]
enum Chunk<'a> {
    Digits(&'a str),
    Text(&'a str),
}

fn chunks(s: &str) -> impl Iterator<Item = Chunk<'_>> {
    let bytes = s.as_bytes();
    let mut i = 0;
    std::iter::from_fn(move || {
        if i >= bytes.len() {
            return None;
        }
        let start = i;
        let digit = bytes[i].is_ascii_digit();
        while i < bytes.len() && bytes[i].is_ascii_digit() == digit {
            i += 1;
        }
        // Boundaries are always ASCII, so slicing is char-safe.
        let part = &s[start..i];
        Some(if digit {
            Chunk::Digits(part)
        } else {
            Chunk::Text(part)
        })
    })
}

fn cmp_digits(a: &str, b: &str) -> Ordering {
    let a_trim = a.trim_start_matches('0');
    let b_trim = b.trim_start_matches('0');
    a_trim
        .len()
        .cmp(&b_trim.len())
        .then_with(|| a_trim.cmp(b_trim))
        // "01" after "1" so ordering is total and stable.
        .then_with(|| a.len().cmp(&b.len()))
}

fn cmp_text(a: &str, b: &str) -> Ordering {
    let la = a.chars().flat_map(char::to_lowercase);
    let lb = b.chars().flat_map(char::to_lowercase);
    la.cmp(lb)
}

/// Compare two names naturally. Ties are broken by the raw string so that the
/// ordering is total (duplicate-looking names still sort deterministically).
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut ia = chunks(a);
    let mut ib = chunks(b);
    loop {
        match (ia.next(), ib.next()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                let ord = match (&x, &y) {
                    (Chunk::Digits(p), Chunk::Digits(q)) => cmp_digits(p, q),
                    (Chunk::Text(p), Chunk::Text(q)) => cmp_text(p, q),
                    (Chunk::Digits(_), Chunk::Text(_)) => Ordering::Less,
                    (Chunk::Text(_), Chunk::Digits(_)) => Ordering::Greater,
                };
                if ord != Ordering::Equal {
                    return ord;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(mut v: Vec<&str>) -> Vec<&str> {
        v.sort_by(|a, b| natural_cmp(a, b));
        v
    }

    #[test]
    fn numbers_sort_numerically() {
        assert_eq!(
            sorted(vec!["10.jpg", "2.jpg", "1.jpg"]),
            vec!["1.jpg", "2.jpg", "10.jpg"]
        );
        assert_eq!(
            sorted(vec!["p010.png", "p002.png", "p001.png"]),
            vec!["p001.png", "p002.png", "p010.png"]
        );
    }

    #[test]
    fn folders_and_case() {
        assert_eq!(
            sorted(vec!["Ch2/1.jpg", "ch10/1.jpg", "ch1/2.jpg", "CH1/10.jpg"]),
            vec!["ch1/2.jpg", "CH1/10.jpg", "Ch2/1.jpg", "ch10/1.jpg"]
        );
    }

    #[test]
    fn duplicates_are_deterministic() {
        assert_eq!(natural_cmp("a/1.jpg", "a/1.jpg"), Ordering::Equal);
        assert_ne!(natural_cmp("1.jpg", "01.jpg"), Ordering::Equal);
        assert_ne!(natural_cmp("A.jpg", "a.jpg"), Ordering::Equal);
    }

    #[test]
    fn huge_numbers_do_not_overflow() {
        let a = "99999999999999999999999999.jpg";
        let b = "100000000000000000000000000.jpg";
        assert_eq!(natural_cmp(a, b), Ordering::Less);
    }
}
