//! Directory listing for the core file browser.
//!
//! Directories can contain far more entries than we can afford to keep in
//! memory at once, so listings are produced as fixed-size windows over the
//! fully sorted directory. Each window is computed in a single pass over the
//! directory, keeping only the entries that fall inside the window.

use std::{
    cmp::Ordering,
    path::{Path, PathBuf},
};

/// Sort key for a directory entry: directories first, then a
/// case-insensitive "natural" ordering (so "Game 2" sorts before "Game 10").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryKey {
    pub is_file: bool,
    folded: String,
    pub name: String,
}

impl EntryKey {
    pub fn new(name: &str, is_file: bool) -> Self {
        EntryKey {
            is_file,
            folded: name.to_lowercase(),
            name: name.to_string(),
        }
    }

    fn stem(&self) -> &str {
        match self.folded.rfind('.') {
            Some(i) if self.is_file && i > 0 => &self.folded[..i],
            _ => &self.folded,
        }
    }
}

impl Ord for EntryKey {
    fn cmp(&self, other: &Self) -> Ordering {
        // Compare file names without their extensions first, so that
        // "Game.gba" sorts before "Game 2.gba".
        self.is_file
            .cmp(&other.is_file)
            .then_with(|| natural_cmp(self.stem(), other.stem()))
            .then_with(|| natural_cmp(&self.folded, &other.folded))
            .then_with(|| self.name.cmp(&other.name))
    }
}

impl PartialOrd for EntryKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Compare two strings, treating runs of ASCII digits as numbers.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.as_bytes(), b.as_bytes());
    loop {
        match (a.first(), b.first()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let a_len = a.iter().take_while(|c| c.is_ascii_digit()).count();
                let b_len = b.iter().take_while(|c| c.is_ascii_digit()).count();
                let (a_num, b_num) = (&a[..a_len], &b[..b_len]);
                // Compare numerically: ignore leading zeros, then longer is bigger.
                let a_trim = trim_zeros(a_num);
                let b_trim = trim_zeros(b_num);
                let ordering = a_trim
                    .len()
                    .cmp(&b_trim.len())
                    .then_with(|| a_trim.cmp(b_trim))
                    .then_with(|| a_len.cmp(&b_len));
                if ordering != Ordering::Equal {
                    return ordering;
                }
                a = &a[a_len..];
                b = &b[b_len..];
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(y);
                }
                a = &a[1..];
                b = &b[1..];
            }
        }
    }
}

fn trim_zeros(digits: &[u8]) -> &[u8] {
    let zeros = digits.iter().take_while(|&&c| c == b'0').count();
    &digits[zeros..]
}

/// Where a listing window is positioned within the sorted directory.
#[derive(Clone, Debug)]
pub enum Anchor {
    /// The first page of the directory.
    Start,
    /// A window containing this entry (if it exists), near the top.
    Around(EntryKey),
    /// The page of entries following this entry.
    After(EntryKey),
    /// The page of entries preceding this entry.
    Before(EntryKey),
}

/// A window of a sorted directory listing.
pub struct Window {
    pub entries: Vec<EntryKey>,
    /// Index of the entry that should be highlighted.
    pub highlight: usize,
    /// True if there are entries before this window.
    pub has_prev: bool,
    /// True if there are entries after this window.
    pub has_next: bool,
}

/// Keeps the `limit` smallest (or largest) keys seen so far, in sorted order.
struct Selection {
    keys: Vec<EntryKey>,
    limit: usize,
    keep_largest: bool,
    seen: usize,
}

impl Selection {
    fn new(limit: usize, keep_largest: bool) -> Self {
        Selection {
            keys: Vec::with_capacity(limit + 1),
            limit,
            keep_largest,
            seen: 0,
        }
    }

    fn push(&mut self, key: EntryKey) {
        self.seen += 1;
        if self.limit == 0 {
            return;
        }
        if self.keys.len() == self.limit {
            let excluded = if self.keep_largest {
                key <= self.keys[0]
            } else {
                key >= self.keys[self.limit - 1]
            };
            if excluded {
                return;
            }
        }
        let pos = self.keys.binary_search(&key).unwrap_or_else(|p| p);
        self.keys.insert(pos, key);
        if self.keys.len() > self.limit {
            if self.keep_largest {
                self.keys.remove(0);
            } else {
                self.keys.pop();
            }
        }
    }

    fn has_more(&self) -> bool {
        self.seen > self.keys.len()
    }
}

/// Compute a window of up to `page_size` entries from a stream of keys.
pub fn window(
    keys: impl Iterator<Item = EntryKey>,
    anchor: &Anchor,
    page_size: usize,
) -> Window {
    let (pivot, before, after) = match anchor {
        Anchor::Start => (None, 0, page_size),
        Anchor::Around(key) => (Some(key), page_size / 4, page_size - page_size / 4),
        // One extra, because the anchor itself may be included.
        Anchor::After(key) => (Some(key), 0, page_size + 1),
        Anchor::Before(key) => (Some(key), page_size, 0),
    };

    let mut below = Selection::new(before, true);
    let mut above = Selection::new(after, false);
    for key in keys {
        match pivot {
            Some(pivot) if key < *pivot => below.push(key),
            _ => above.push(key),
        }
    }

    let mut has_prev = below.has_more();
    let mut has_next = above.has_more();
    if let Anchor::After(key) = anchor {
        if above.keys.first() == Some(key) {
            above.keys.remove(0);
        } else {
            above.keys.truncate(page_size);
        }
        has_prev = true;
    }
    if let Anchor::Before(_) = anchor {
        has_next = above.seen > 0;
    }

    let highlight = match anchor {
        Anchor::Start | Anchor::After(_) => 0,
        Anchor::Around(_) => below.keys.len(),
        Anchor::Before(_) => below.keys.len().saturating_sub(1),
    };

    let mut entries = below.keys;
    entries.extend(above.keys);
    let highlight = highlight.min(entries.len().saturating_sub(1));
    Window {
        entries,
        highlight,
        has_prev,
        has_next,
    }
}

/// Iterate over the eligible entries of a directory.
///
/// Hidden files are skipped, and files must have one of the given extensions
/// (case-insensitive).
pub fn read_dir_keys<'a>(
    path: &Path,
    extensions: &'a [&'a str],
) -> std::io::Result<impl Iterator<Item = EntryKey> + 'a> {
    Ok(path.read_dir()?.filter_map(move |e| {
        let e = e.ok()?;
        let name = e.file_name();
        let name = name.to_str()?;
        if name.starts_with('.') {
            return None;
        }
        let is_file = e.metadata().ok()?.file_type().is_file();
        if is_file && !has_extension(name, extensions) {
            return None;
        }
        Some(EntryKey::new(name, is_file))
    }))
}

/// Whether a file name ends with one of the extensions (case-insensitive).
pub fn has_extension(name: &str, extensions: &[&str]) -> bool {
    let name = name.as_bytes();
    extensions.iter().any(|ext| {
        let ext = ext.as_bytes();
        name.len() >= ext.len() && name[name.len() - ext.len()..].eq_ignore_ascii_case(ext)
    })
}

/// What a file browser entry refers to.
#[derive(Clone, Debug)]
pub enum Target {
    /// A file or directory.
    Path(PathBuf),
    /// The virtual favorites directory.
    Favorites,
    /// The previous page of the current directory.
    PrevPage,
    /// The next page of the current directory.
    NextPage,
}

/// An entry shown in the file browser.
pub struct Entry {
    pub label: String,
    pub is_dir: bool,
    pub target: Target,
}
