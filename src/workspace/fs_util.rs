//! Filesystem helpers shared by the workspace, the explorer and the document
//! providers.
//!
//! Everything here is synchronous and pure; callers run it on the background
//! executor. The two things this module owns that are easy to get wrong are
//! **ordering** (a listing that jumps around between refreshes reads as a bug)
//! and **path identity** (Windows hands out `\\?\` prefixes and mixes
//! drive-letter case, so the same file can spell itself several ways).

use std::{
    cmp::Ordering,
    ffi::OsStr,
    io,
    path::{Component, Path, PathBuf},
};

/// How many entries of one directory are listed before the rest are dropped.
///
/// A directory with more children than this is not browsable in a side panel
/// anyway, and an uncapped listing is how the UI thread ends up frozen on a
/// network share (risk R8).
pub const MAX_DIR_ENTRIES: usize = 5_000;

/// The largest file the text viewer will open. Bigger files get an explanatory
/// panel instead of a buffer that takes seconds to lay out.
pub const MAX_TEXT_BYTES: u64 = 8 * 1024 * 1024;

/// How much of a file is read to decide whether it is binary.
pub const BINARY_SNIFF_BYTES: usize = 8 * 1024;

/// Directory names never shown in the explorer, and whose events are ignored.
///
/// These are the three that make a source tree unreadable in practice. A
/// `.gitignore`-aware filter is a later refinement; see [`super::watcher`].
const IGNORED_DIRS: [&str; 3] = ["target", "node_modules", ".git"];

/// One directory entry, as the tree needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntryInfo {
    pub path: PathBuf,
    /// The file name, for display. Kept separate from `path` because the tree
    /// shows only the leaf and the full path is not the label.
    pub name: String,
    pub is_dir: bool,
}

impl DirEntryInfo {
    /// Whether this entry's name marks it as one the explorer skips.
    pub fn is_ignored(&self) -> bool {
        is_ignored_name(&self.name)
    }
}

/// Whether `name` is a hidden or vendored entry the explorer leaves out.
///
/// Hidden means "starts with a dot", which covers `.git`, `.venv` and the rest
/// on every platform; Windows' hidden *attribute* is deliberately not consulted
/// because it is not set on the dot-directories developers actually mean.
pub fn is_ignored_name(name: &str) -> bool {
    name.starts_with('.') || IGNORED_DIRS.contains(&name)
}

/// Whether a path, or any of its ancestors below `root`, is ignored.
///
/// Used by the watcher, which sees absolute paths rather than names.
pub fn is_ignored_path(path: &Path, root: &Path) -> bool {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .any(|component| match component {
            Component::Normal(name) => name.to_str().is_some_and(is_ignored_name),
            _ => false,
        })
}

/// Read one directory: directories first, then files, each group sorted
/// case-insensitively by name.
///
/// The order is part of the contract — a listing whose order changes between
/// refreshes makes the tree appear to shuffle — so it is fixed here rather than
/// left to the filesystem. Returns at most [`MAX_DIR_ENTRIES`] entries and a
/// flag saying whether the listing was cut short.
pub fn read_dir_sorted(dir: &Path) -> io::Result<(Vec<DirEntryInfo>, bool)> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        // A single unreadable entry (a dangling symlink, a permission we lack)
        // must not fail the whole listing, or one bad file hides its siblings.
        let Ok(entry) = entry else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_ignored_name(&name) {
            continue;
        }
        let path = entry.path();
        let is_dir = entry_is_dir(&entry, &path);
        entries.push(DirEntryInfo { path, name, is_dir });
    }

    entries.sort_by(compare_entries);

    let truncated = entries.len() > MAX_DIR_ENTRIES;
    entries.truncate(MAX_DIR_ENTRIES);
    Ok((entries, truncated))
}

/// Resolve whether an entry is a directory, following symlinks.
///
/// `DirEntry::file_type` does not follow symlinks, so a link to a directory
/// would be listed as a file and could not be expanded.
fn entry_is_dir(entry: &std::fs::DirEntry, path: &Path) -> bool {
    match entry.file_type() {
        Ok(kind) if kind.is_symlink() => std::fs::metadata(path).is_ok_and(|meta| meta.is_dir()),
        Ok(kind) => kind.is_dir(),
        Err(_) => std::fs::metadata(path).is_ok_and(|meta| meta.is_dir()),
    }
}

/// Directories first, then case-insensitive name, then exact name.
///
/// The final comparison on the raw name keeps the order total: two names that
/// differ only by case would otherwise compare equal and sort unstably.
fn compare_entries(left: &DirEntryInfo, right: &DirEntryInfo) -> Ordering {
    right
        .is_dir
        .cmp(&left.is_dir)
        .then_with(|| compare_names(&left.name, &right.name))
}

/// Compare two display names the way a user reads them.
fn compare_names(left: &str, right: &str) -> Ordering {
    let folded = left.to_lowercase().cmp(&right.to_lowercase());
    folded.then_with(|| left.cmp(right))
}

/// A stable key for comparing two paths that name the same file.
///
/// On Windows this strips the `\\?\` verbatim prefix that `canonicalize`
/// produces, maps `/` to `\`, and folds case, because `C:\Foo` and `c:\foo` are
/// one file. On other platforms it is the path as written. The result is for
/// comparison only — never open a file with it, since folding case destroys the
/// real name.
pub fn path_key(path: &Path) -> String {
    let text = path.to_string_lossy();
    let stripped = strip_verbatim_prefix(&text);
    if cfg!(windows) {
        stripped.replace('/', "\\").to_lowercase()
    } else {
        stripped.into_owned()
    }
}

/// Whether two paths name the same file, by [`path_key`].
pub fn same_path(left: &Path, right: &Path) -> bool {
    path_key(left) == path_key(right)
}

/// Drop the `\\?\` (or `//?/`) prefix `canonicalize` adds on Windows.
fn strip_verbatim_prefix(text: &str) -> std::borrow::Cow<'_, str> {
    if let Some(rest) = text
        .strip_prefix(r"\\?\")
        .or_else(|| text.strip_prefix("//?/"))
    {
        // `\\?\UNC\server\share` is the verbatim spelling of a network path;
        // dropping only the prefix would leave `UNC\`, which is not a path.
        if let Some(unc) = rest
            .strip_prefix("UNC\\")
            .or_else(|| rest.strip_prefix("UNC/"))
        {
            return std::borrow::Cow::Owned(format!(r"\\{unc}"));
        }
        return std::borrow::Cow::Owned(rest.to_owned());
    }
    std::borrow::Cow::Borrowed(text)
}

/// Whether a byte sample looks binary.
///
/// The test is a NUL byte in the sample, which is what "binary" means for the
/// purpose that matters here: deciding whether a file can be put in a text
/// buffer. UTF-16 text contains NULs and is therefore reported binary; that is
/// the conservative answer, since opening it as UTF-8 would produce garbage.
pub fn is_binary_sample(bytes: &[u8]) -> bool {
    bytes.contains(&0)
}

/// Read the head of a file and report whether it is binary.
pub fn is_binary_file(path: &Path) -> io::Result<bool> {
    use std::io::Read as _;
    let mut file = std::fs::File::open(path)?;
    let mut head = [0u8; BINARY_SNIFF_BYTES];
    let read = file.read(&mut head)?;
    Ok(is_binary_sample(&head[..read]))
}

/// The lowercased extension of `path`, without the dot.
pub fn extension_of(path: &Path) -> Option<String> {
    path.extension()
        .and_then(OsStr::to_str)
        .map(str::to_lowercase)
        .filter(|ext| !ext.is_empty())
}

/// The last path component, for display.
pub fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a throwaway directory under the system temp dir.
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lllearning-test-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create the test directory");
        dir
    }

    fn touch(path: &Path) {
        std::fs::write(path, b"x").expect("write the test file");
    }

    #[test]
    fn read_dir_orders_dirs_first_and_skips_ignored() {
        let dir = temp_dir("order");
        std::fs::create_dir(dir.join("zeta")).unwrap();
        std::fs::create_dir(dir.join("Alpha")).unwrap();
        std::fs::create_dir(dir.join("target")).unwrap();
        std::fs::create_dir(dir.join(".git")).unwrap();
        std::fs::create_dir(dir.join("node_modules")).unwrap();
        touch(&dir.join("b.txt"));
        touch(&dir.join("A.txt"));
        touch(&dir.join(".hidden"));

        let (entries, truncated) = read_dir_sorted(&dir).unwrap();
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();

        assert_eq!(
            names,
            vec!["Alpha", "zeta", "A.txt", "b.txt"],
            "directories first, each group case-insensitively sorted, ignored names gone"
        );
        assert!(!truncated);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_symlinked_directory_is_listed_as_a_directory() {
        let dir = temp_dir("symlink");
        std::fs::create_dir(dir.join("real")).unwrap();
        let link = dir.join("link");

        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(dir.join("real"), &link).is_ok();
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(dir.join("real"), &link).is_ok();

        if made {
            let (entries, _) = read_dir_sorted(&dir).unwrap();
            let link = entries
                .iter()
                .find(|entry| entry.name == "link")
                .expect("the link is listed");
            assert!(link.is_dir, "a symlink to a directory must expand");
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn path_keys_fold_case_on_windows_only() {
        let upper = Path::new(r"C:\Codes\Foo\a.txt");
        let lower = Path::new(r"c:\codes\foo\A.TXT");

        if cfg!(windows) {
            assert_eq!(path_key(upper), path_key(lower));
            assert!(same_path(upper, lower));
        } else {
            assert_ne!(path_key(upper), path_key(lower));
        }
    }

    #[test]
    fn path_keys_strip_the_verbatim_prefix() {
        assert_eq!(
            path_key(Path::new(r"\\?\C:\a\b")),
            path_key(Path::new(r"C:\a\b"))
        );
        assert_eq!(
            path_key(Path::new(r"\\?\UNC\server\share\f")),
            path_key(Path::new(r"\\server\share\f"))
        );
    }

    #[test]
    fn separators_compare_equal_on_windows() {
        if cfg!(windows) {
            assert_eq!(
                path_key(Path::new("C:/a/b")),
                path_key(Path::new(r"C:\a\b"))
            );
        }
    }

    #[test]
    fn binary_sniff_looks_for_nul() {
        assert!(!is_binary_sample(b"hello, world\n"));
        assert!(!is_binary_sample(b""));
        assert!(is_binary_sample(b"ab\0cd"));
        // UTF-8 non-ASCII is text even though the bytes are high.
        assert!(!is_binary_sample("héllo — ünïcode".as_bytes()));
    }

    #[test]
    fn extensions_are_lowercased_and_empty_is_none() {
        assert_eq!(
            extension_of(Path::new("a/b/README.MD")).as_deref(),
            Some("md")
        );
        assert_eq!(extension_of(Path::new("a/b/Makefile")), None);
        assert_eq!(extension_of(Path::new("a/b/.bashrc")), None);
    }

    #[test]
    fn ignored_paths_are_found_below_the_root() {
        let root = Path::new("/work");
        assert!(is_ignored_path(Path::new("/work/target/debug/x"), root));
        assert!(is_ignored_path(Path::new("/work/.git/config"), root));
        assert!(!is_ignored_path(Path::new("/work/src/main.rs"), root));
        // The root itself is never ignored, even when it is called `target`.
        assert!(!is_ignored_path(Path::new("/work"), Path::new("/work")));
    }
}
