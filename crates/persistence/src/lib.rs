//! Progress and per-book preferences (spec §6, §23.4).
//!
//! * One small JSON file in the app's own data directory. Books are never
//!   written to.
//! * Writes are atomic: temp file in the same directory → `fsync` → `rename`
//!   → `fsync(dir)`. A crash leaves either the old or the new state.
//! * A corrupt state file is moved aside (never deleted) and the app starts
//!   with empty state.
//! * Records for books that went missing are kept so progress can be relinked.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub const STATE_VERSION: u32 = 1;
const STATE_FILE: &str = "state.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    LeftToRight,
    /// Manga default.
    #[default]
    RightToLeft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    #[default]
    Page,
    Width,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContrastPref {
    #[default]
    Normal,
    Darker,
}

/// The only per-book settings in v1 (spec §23.4) + hidden contrast.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReadingPrefs {
    pub fit: Fit,
    pub direction: Direction,
    pub show_progress: bool,
    pub contrast: ContrastPref,
}

impl Default for ReadingPrefs {
    fn default() -> Self {
        Self {
            fit: Fit::Page,
            direction: Direction::RightToLeft,
            show_progress: true,
            contrast: ContrastPref::Normal,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookRecord {
    pub id: String,
    pub path: PathBuf,
    pub title: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub page_count: u32,
    pub last_read_page: u32,
    /// Unix seconds; 0 = never opened.
    pub last_opened_at: u64,
    #[serde(default)]
    pub prefs: ReadingPrefs,
    /// File size and mtime used for the id; kept for relinking.
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub mtime: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    pub version: u32,
    pub books: BTreeMap<String, BookRecord>,
    /// Defaults applied to newly opened books.
    #[serde(default)]
    pub default_prefs: ReadingPrefs,
}

impl State {
    /// Most recently opened book, for the "Continue" card.
    pub fn last_opened(&self) -> Option<&BookRecord> {
        self.books
            .values()
            .filter(|b| b.last_opened_at > 0)
            .max_by_key(|b| b.last_opened_at)
    }

    /// Recently opened books, newest first.
    pub fn recent(&self, n: usize) -> Vec<&BookRecord> {
        let mut v: Vec<_> = self
            .books
            .values()
            .filter(|b| b.last_opened_at > 0)
            .collect();
        v.sort_by_key(|b| std::cmp::Reverse(b.last_opened_at));
        v.truncate(n);
        v
    }
}

/// Stable book id: FNV-1a 64 over canonical path + size + mtime.
///
/// Cheap (no content hashing of multi-GB files on launch). A rename gives a
/// new id; [`Store::relink`] can move progress across.
pub fn book_id(path: &Path, size: u64, mtime: u64) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for b in bytes {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    feed(path.to_string_lossy().as_bytes());
    feed(&[0]);
    feed(&size.to_le_bytes());
    feed(&mtime.to_le_bytes());
    format!("{h:016x}")
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Write `bytes` to `path` atomically (same-directory temp + rename).
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        file_name.to_string_lossy(),
        std::process::id()
    ));
    {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    // Persist the rename itself. Not all filesystems (e.g. FAT/vfat on some
    // Kindles' user storage) support fsync on directories; ignore that error.
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

#[derive(Debug)]
pub enum LoadOutcome {
    Fresh,
    Loaded,
    /// Existing file was unreadable and moved to this path.
    RecoveredFromCorrupt(PathBuf),
}

/// State + debounced atomic persistence.
pub struct Store {
    dir: PathBuf,
    state: State,
    dirty_since: Option<Instant>,
    debounce: Duration,
}

impl Store {
    pub fn open(dir: impl Into<PathBuf>) -> io::Result<(Self, LoadOutcome)> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        let path = dir.join(STATE_FILE);
        let (state, outcome) = match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<State>(&bytes) {
                Ok(s) if s.version <= STATE_VERSION => (s, LoadOutcome::Loaded),
                _ => {
                    let aside = dir.join(format!("state.corrupt-{}.json", unix_now()));
                    fs::rename(&path, &aside)?;
                    (State::default(), LoadOutcome::RecoveredFromCorrupt(aside))
                }
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => (State::default(), LoadOutcome::Fresh),
            Err(e) => return Err(e),
        };
        let mut state = state;
        state.version = STATE_VERSION;
        Ok((
            Self {
                dir,
                state,
                dirty_since: None,
                debounce: Duration::from_millis(1500),
            },
            outcome,
        ))
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join(STATE_FILE)
    }

    pub fn book(&self, id: &str) -> Option<&BookRecord> {
        self.state.books.get(id)
    }

    /// Mutate state; marks it dirty for the next debounced flush.
    pub fn update<R>(&mut self, f: impl FnOnce(&mut State) -> R) -> R {
        let r = f(&mut self.state);
        self.dirty_since.get_or_insert_with(Instant::now);
        r
    }

    /// Insert or refresh a catalog entry without touching progress.
    pub fn upsert_book(&mut self, id: &str, path: &Path, title: &str, size: u64, mtime: u64) {
        let defaults = self.state.default_prefs;
        self.update(|s| {
            let rec = s.books.entry(id.to_owned()).or_insert_with(|| BookRecord {
                id: id.to_owned(),
                path: path.to_owned(),
                title: title.to_owned(),
                kind: "cbz".into(),
                page_count: 0,
                last_read_page: 0,
                last_opened_at: 0,
                prefs: defaults,
                size,
                mtime,
            });
            rec.path = path.to_owned();
            rec.title = title.to_owned();
        });
    }

    /// Move progress from a vanished record to a new id (renamed/updated file).
    pub fn relink(&mut self, old_id: &str, new_id: &str) -> bool {
        self.update(|s| {
            let (Some(old), Some(new)) = (s.books.get(old_id).cloned(), s.books.get_mut(new_id))
            else {
                return false;
            };
            new.last_read_page = old.last_read_page;
            new.last_opened_at = old.last_opened_at;
            new.prefs = old.prefs;
            s.books.remove(old_id);
            true
        })
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty_since.is_some()
    }

    /// Flush if dirty for longer than the debounce interval.
    pub fn flush_if_due(&mut self) -> io::Result<bool> {
        match self.dirty_since {
            Some(t) if t.elapsed() >= self.debounce => self.flush().map(|_| true),
            _ => Ok(false),
        }
    }

    /// Flush now (exit/suspend). No-op when clean.
    pub fn flush(&mut self) -> io::Result<()> {
        if self.dirty_since.is_none() {
            return Ok(());
        }
        let bytes = serde_json::to_vec_pretty(&self.state).map_err(io::Error::other)?;
        atomic_write(&self.path(), &bytes)?;
        self.dirty_since = None;
        Ok(())
    }

    pub fn debounce(&self) -> Duration {
        self.debounce
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("eink-persist-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn roundtrip_progress_and_prefs() {
        let dir = tmpdir("rt");
        let (mut s, o) = Store::open(&dir).unwrap();
        assert!(matches!(o, LoadOutcome::Fresh));
        let id = book_id(Path::new("/mnt/us/a.cbz"), 10, 20);
        s.upsert_book(&id, Path::new("/mnt/us/a.cbz"), "A", 10, 20);
        s.update(|st| {
            let b = st.books.get_mut(&id).unwrap();
            b.last_read_page = 41;
            b.page_count = 230;
            b.last_opened_at = 5;
            b.prefs.direction = Direction::LeftToRight;
        });
        s.flush().unwrap();
        assert!(!s.is_dirty());
        let (s2, o) = Store::open(&dir).unwrap();
        assert!(matches!(o, LoadOutcome::Loaded));
        let b = s2.book(&id).unwrap();
        assert_eq!(b.last_read_page, 41);
        assert_eq!(b.prefs.direction, Direction::LeftToRight);
        assert_eq!(s2.state().last_opened().unwrap().id, id);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn upsert_keeps_progress() {
        let dir = tmpdir("upsert");
        let (mut s, _) = Store::open(&dir).unwrap();
        s.upsert_book("x", Path::new("/a"), "A", 1, 1);
        s.update(|st| st.books.get_mut("x").unwrap().last_read_page = 7);
        s.upsert_book("x", Path::new("/a"), "A renamed", 1, 1);
        assert_eq!(s.book("x").unwrap().last_read_page, 7);
        assert_eq!(s.book("x").unwrap().title, "A renamed");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn corrupt_state_is_moved_aside_not_deleted() {
        let dir = tmpdir("corrupt");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(STATE_FILE), b"{ not json").unwrap();
        let (s, o) = Store::open(&dir).unwrap();
        assert!(s.state().books.is_empty());
        let LoadOutcome::RecoveredFromCorrupt(p) = o else {
            panic!()
        };
        assert_eq!(fs::read(p).unwrap(), b"{ not json");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp() {
        let dir = tmpdir("atomic");
        let p = dir.join("f.json");
        atomic_write(&p, b"one").unwrap();
        atomic_write(&p, b"two").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"two");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn debounce() {
        let dir = tmpdir("debounce");
        let (mut s, _) = Store::open(&dir).unwrap();
        s.debounce = Duration::from_millis(30);
        s.update(|st| st.default_prefs.show_progress = false);
        assert!(!s.flush_if_due().unwrap());
        std::thread::sleep(Duration::from_millis(40));
        assert!(s.flush_if_due().unwrap());
        assert!(s.path().exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn relink_moves_progress() {
        let dir = tmpdir("relink");
        let (mut s, _) = Store::open(&dir).unwrap();
        s.upsert_book("old", Path::new("/a"), "A", 1, 1);
        s.update(|st| {
            let b = st.books.get_mut("old").unwrap();
            b.last_read_page = 9;
            b.last_opened_at = 3;
        });
        s.upsert_book("new", Path::new("/b"), "A", 1, 2);
        assert!(s.relink("old", "new"));
        assert_eq!(s.book("new").unwrap().last_read_page, 9);
        assert!(s.book("old").is_none());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ids_are_stable_and_distinct() {
        let a = book_id(Path::new("/x.cbz"), 1, 2);
        assert_eq!(a, book_id(Path::new("/x.cbz"), 1, 2));
        assert_ne!(a, book_id(Path::new("/x.cbz"), 1, 3));
        assert_eq!(a.len(), 16);
    }

    #[test]
    fn unknown_fields_and_missing_prefs_are_tolerated() {
        let dir = tmpdir("compat");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(STATE_FILE),
            br#"{"version":1,"books":{"a":{"id":"a","path":"/a","title":"A","type":"cbz","page_count":3,"last_read_page":2,"last_opened_at":1,"future":true}}}"#,
        )
        .unwrap();
        let (s, o) = Store::open(&dir).unwrap();
        assert!(matches!(o, LoadOutcome::Loaded));
        assert_eq!(s.book("a").unwrap().prefs, ReadingPrefs::default());
        fs::remove_dir_all(dir).unwrap();
    }
}
