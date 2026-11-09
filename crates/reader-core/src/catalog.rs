//! Book folder scan. Reads directory metadata only; never opens archives.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use cbz_reader::natural_cmp;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    pub id: String,
    pub path: PathBuf,
    pub title: String,
    pub size: u64,
    pub mtime: u64,
}

fn is_book(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("cbz"))
}

/// Human-friendly title from the file name.
pub fn title_for(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let t = stem.replace('_', " ");
    let t = t.trim();
    if t.is_empty() {
        "Untitled".into()
    } else {
        t.to_owned()
    }
}

/// Scan `dir` (and sub-folders up to `max_depth`) for `.cbz` files.
///
/// Hidden entries are skipped, symlinks are not followed and the number of
/// results is capped so a huge folder cannot stall the UI indefinitely.
pub fn scan(
    dir: &Path,
    max_depth: usize,
    max_entries: usize,
) -> std::io::Result<Vec<CatalogEntry>> {
    let mut out = Vec::new();
    let mut stack = vec![(dir.to_owned(), 0usize)];
    while let Some((d, depth)) = stack.pop() {
        let rd = match std::fs::read_dir(&d) {
            Ok(rd) => rd,
            Err(e) if d == dir => return Err(e),
            Err(_) => continue,
        };
        for entry in rd.flatten() {
            if out.len() >= max_entries {
                break;
            }
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            let Ok(ft) = entry.file_type() else { continue };
            let path = entry.path();
            if ft.is_dir() {
                if depth < max_depth {
                    stack.push((path, depth + 1));
                }
            } else if ft.is_file() && is_book(&path) {
                let Ok(meta) = entry.metadata() else { continue };
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                out.push(CatalogEntry {
                    id: persistence::book_id(&path, meta.len(), mtime),
                    title: title_for(&path),
                    path,
                    size: meta.len(),
                    mtime,
                });
            }
        }
    }
    out.sort_by(|a, b| natural_cmp(&a.title, &b.title));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_cbz_only_and_sorts() {
        let d = std::env::temp_dir().join(format!("eink-cat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("series/deep/deeper")).unwrap();
        std::fs::create_dir_all(d.join(".hidden")).unwrap();
        for p in [
            "Vol_10.cbz",
            "Vol_2.CBZ",
            "notes.txt",
            "series/Ch 1.cbz",
            "series/deep/deeper/too-deep.cbz",
            ".hidden/x.cbz",
            "book.epub",
        ] {
            std::fs::write(d.join(p), b"x").unwrap();
        }
        let titles: Vec<_> = scan(&d, 2, 100)
            .unwrap()
            .into_iter()
            .map(|e| e.title)
            .collect();
        assert_eq!(titles, ["Ch 1", "Vol 2", "Vol 10"]);
        assert_eq!(scan(&d, 0, 1).unwrap().len(), 1);
        assert!(scan(&d.join("missing"), 1, 10).is_err());
        std::fs::remove_dir_all(d).unwrap();
    }
}
