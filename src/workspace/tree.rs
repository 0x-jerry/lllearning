//! The explorer's lazy directory cache, and the translation from it to
//! `TreeItem`s.
//!
//! The cache holds one entry per directory that has ever been read, keyed by
//! its canonical path. Nothing is read until a directory is expanded, so a
//! workspace with a deep tree costs one `read_dir` at startup.
//!
//! # Why expansion lives here and not in `TreeItem`
//!
//! `TreeState::set_items` rebuilds the flat entry list from scratch, and a
//! rebuilt `TreeItem` carries whatever expansion flag it was constructed with.
//! `TreeItem`'s flag is `Rc<RefCell<..>>` shared with clones, but the *tree* is
//! what a rebuild consumes, so any node the cache hands out must already have
//! the right flag. This module is therefore the single owner of expansion
//! state; the tree is rendered from it, never the other way round. That is also
//! what makes risk R5 (a rebuild losing the selection and the open folders)
//! tractable.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use gpui::{SharedString, base::TreeItem};

use super::fs_util::{DirEntryInfo, file_name_of, read_dir_sorted};

/// A directory whose children have been read.
#[derive(Clone, Debug, Default)]
struct LoadedDir {
    children: Vec<DirEntryInfo>,
}

/// The explorer's view of the workspace folder.
#[derive(Default)]
pub struct DirTree {
    root: Option<PathBuf>,
    /// Directory path -> its listing, for every directory read so far.
    loaded: HashMap<PathBuf, LoadedDir>,
    /// Directories the user has opened, by path.
    expanded: HashSet<PathBuf>,
    /// Paths the user has selected, so a rebuild can restore the highlight.
    selected: Option<PathBuf>,
    /// Directories whose listing was too long, for the explorer to mark.
    truncated: HashSet<PathBuf>,
}

impl DirTree {
    /// A tree with no root: the "no folder open" state.
    pub fn new() -> Self {
        Self::default()
    }

    /// The workspace folder, if one is open.
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// The highlighted entry, if any.
    pub fn selected(&self) -> Option<&Path> {
        self.selected.as_deref()
    }

    /// Record the highlighted entry without touching the disk.
    pub fn set_selected(&mut self, path: Option<PathBuf>) {
        self.selected = path;
    }

    /// Record what a click in the tree did: the row it landed on, and whether
    /// that row was opened or closed.
    ///
    /// The click's effect lives in the `TreeState` first — it toggles the row
    /// and emits the event before the cache hears about it — so the cache has
    /// to be told the new state rather than flipping its own, or the two
    /// disagree and the next click closes what looks open.
    pub fn set_expanded(&mut self, dir: &Path, expanded: bool) {
        if expanded {
            self.expanded.insert(dir.to_path_buf());
        } else {
            self.expanded.remove(dir);
        }
        self.selected = Some(dir.to_path_buf());
    }

    /// Whether a directory's listing was capped.
    pub fn is_truncated(&self, path: &Path) -> bool {
        self.truncated.contains(path)
    }

    /// Whether a directory has been read.
    pub fn is_loaded(&self, path: &Path) -> bool {
        self.loaded.contains_key(path)
    }

    /// Whether a directory is open in the tree.
    pub fn is_expanded(&self, path: &Path) -> bool {
        self.expanded.contains(path)
    }

    /// Start a new workspace at `root`, dropping everything cached for the old
    /// one.
    ///
    /// The root itself is read immediately: without its children the tree has
    /// nothing to show, and one `read_dir` is not worth deferring.
    pub fn set_root(&mut self, root: PathBuf) -> std::io::Result<()> {
        self.reset();
        let (children, truncated) = read_dir_sorted(&root)?;
        self.root = Some(root.clone());
        self.loaded.insert(root.clone(), LoadedDir { children });
        self.expanded.insert(root.clone());
        if truncated {
            self.truncated.insert(root);
        }
        Ok(())
    }

    /// Drop the root and every cached listing.
    pub fn reset(&mut self) {
        self.root = None;
        self.loaded.clear();
        self.expanded.clear();
        self.selected = None;
        self.truncated.clear();
    }

    /// Read a directory's children into the cache, unless already there.
    ///
    /// Returns whether anything changed, which is what tells the caller whether
    /// the tree needs rebuilding. An unreadable directory is cached as empty so
    /// that a second attempt does not retry a known-bad path on every expand.
    pub fn load_children(&mut self, dir: &Path) -> bool {
        if self.loaded.contains_key(dir) {
            return false;
        }
        let loaded = match read_dir_sorted(dir) {
            Ok((children, truncated)) => {
                if truncated {
                    self.truncated.insert(dir.to_path_buf());
                }
                LoadedDir { children }
            }
            Err(_) => LoadedDir::default(),
        };
        self.loaded.insert(dir.to_path_buf(), loaded);
        true
    }

    /// Forget a directory's listing so the next expand re-reads it.
    ///
    /// Used by the watcher when a directory's contents change underneath us.
    pub fn invalidate(&mut self, dir: &Path) {
        self.loaded.remove(dir);
        self.truncated.remove(dir);
    }

    /// Re-read a directory that is already cached, keeping it cached.
    ///
    /// Unlike [`Self::invalidate`] this leaves the expansion state alone, so the
    /// tree does not flicker closed while its contents are refreshed.
    pub fn refresh(&mut self, dir: &Path) -> bool {
        if !self.loaded.contains_key(dir) {
            return false;
        }
        self.invalidate(dir);
        self.load_children(dir)
    }

    /// Forget every cached listing, keeping the root and what is open.
    ///
    /// The refresh button's action: it re-reads on demand rather than up front.
    pub fn invalidate_all(&mut self) {
        let root = self.root.clone();
        let expanded = std::mem::take(&mut self.expanded);
        self.loaded.clear();
        self.truncated.clear();
        self.expanded = expanded;
        if let Some(root) = root {
            self.expanded.insert(root);
        }
    }

    /// Read a directory on the current thread and return it as a listing, for
    /// the caller to install with [`Self::insert_children`].
    ///
    /// Static rather than a method so a background task can call it without
    /// holding the tree, which lives on the UI thread.
    pub fn read_listing(dir: &Path) -> (PathBuf, Vec<DirEntryInfo>, bool) {
        match read_dir_sorted(dir) {
            Ok((children, truncated)) => (dir.to_path_buf(), children, truncated),
            Err(_) => (dir.to_path_buf(), Vec::new(), false),
        }
    }

    /// Mark a directory open without toggling, for "reveal" and for restore.
    pub fn expand(&mut self, dir: &Path) {
        self.expanded.insert(dir.to_path_buf());
    }

    /// Open every ancestor of `path` below the root, and select it.
    ///
    /// This is how a restored document re-opens the folders leading to it. It
    /// only marks directories expanded; the caller still has to load the
    /// children it wants visible.
    pub fn reveal(&mut self, path: &Path) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let Ok(relative) = path.strip_prefix(&root) else {
            self.selected = Some(path.to_path_buf());
            return;
        };
        let mut walked = root;
        for component in relative
            .components()
            .take(relative.components().count().saturating_sub(1))
        {
            walked = walked.join(component);
            self.expanded.insert(walked.clone());
        }
        self.selected = Some(path.to_path_buf());
    }

    /// Every directory that is open, root included.
    ///
    /// The refresh button re-reads these; the list is owned so the caller can
    /// hand it to a background task while the tree stays borrowable.
    pub fn open_dirs(&self) -> Vec<PathBuf> {
        let mut dirs: Vec<PathBuf> = self.expanded.iter().cloned().collect();
        dirs.sort();
        dirs
    }

    /// Install a listing that was read elsewhere, usually on the background
    /// executor.
    ///
    /// Merging rather than reading here is what keeps `read_dir` off the UI
    /// thread (risk R8): the caller reads on a background thread and hands the
    /// result back.
    pub fn insert_children(&mut self, dir: PathBuf, children: Vec<DirEntryInfo>, truncated: bool) {
        if truncated {
            self.truncated.insert(dir.clone());
        }
        self.loaded.insert(dir, LoadedDir { children });
    }

    /// Install a batch of listings read elsewhere.
    pub fn insert_listings(&mut self, listings: Vec<(PathBuf, Vec<DirEntryInfo>, bool)>) {
        for (dir, children, truncated) in listings {
            self.insert_children(dir, children, truncated);
        }
    }

    /// Every directory that is open and whose children are not cached.
    ///
    /// The caller loads these; the workspace uses it to restore a session's
    /// open folders without walking the disk synchronously in one burst.
    pub fn expanded_without_children(&self) -> Vec<PathBuf> {
        self.expanded
            .iter()
            .filter(|dir| !self.loaded.contains_key(*dir))
            .cloned()
            .collect()
    }

    /// The tree, ready for `TreeState::set_items`.
    ///
    /// The root's *children* are the top level, not the root: the folder is
    /// already named in the toolbar above the tree, and a single always-open
    /// row around everything would cost a level of indentation and a row that
    /// can never be clicked away.
    ///
    /// Each directory that has been read gets its children as `TreeItem`s; an
    /// expanded directory is built with `expanded(true)` so the tree shows its
    /// contents. Directories not yet read appear as leaves, which is what makes
    /// expansion lazy.
    pub fn items(&self) -> Vec<TreeItem> {
        let Some(root) = &self.root else {
            return Vec::new();
        };
        self.children_of(root)
    }

    /// The `TreeItem`s for a directory's listing, empty if it was never read.
    fn children_of(&self, dir: &Path) -> Vec<TreeItem> {
        self.loaded
            .get(dir)
            .map(|loaded| {
                loaded
                    .children
                    .iter()
                    .map(|entry| self.item_for(&entry.path))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Build the `TreeItem` for `path`, recursing only into open directories.
    fn item_for(&self, path: &Path) -> TreeItem {
        let name = file_name_of(path);
        let is_dir = self.is_dir(path);
        let expanded = is_dir && self.expanded.contains(path);

        let item = TreeItem::new(id_for(path), SharedString::from(name));

        if !is_dir {
            return item;
        }
        if !expanded {
            // A closed directory keeps a placeholder child so `is_folder()` —
            // `!children.is_empty()` — stays true and the row is expandable.
            // Risk R3: without it, an unread directory cannot be opened at all.
            return item.child(placeholder_child());
        }

        match self.loaded.get(path) {
            Some(loaded) if !loaded.children.is_empty() => {
                let children = self.children_of(path);
                item.children(children).expanded(true)
            }
            // An open directory with nothing in it still needs the placeholder,
            // or the row stops being a folder the moment it is expanded.
            _ => item.child(placeholder_child()).expanded(true),
        }
    }

    /// Whether `path` is a directory according to the cache.
    ///
    /// Directories that were never read are only known from their parent's
    /// listing, which is exactly where this looks.
    fn is_dir(&self, path: &Path) -> bool {
        if self.loaded.contains_key(path) {
            return true;
        }
        path.parent()
            .and_then(|parent| self.loaded.get(parent))
            .is_some_and(|loaded| {
                loaded
                    .children
                    .iter()
                    .any(|entry| entry.path == path && entry.is_dir)
            })
    }

    /// Resolve a tree item id back to a path.
    pub fn path_for_id(id: &SharedString) -> PathBuf {
        PathBuf::from(id.as_ref())
    }

    /// Whether `id` names the placeholder row rather than a real entry.
    pub fn is_placeholder_id(id: &SharedString) -> bool {
        id.as_ref() == PLACEHOLDER_ID
    }
}

/// The id every placeholder child carries. It is not a path, so no real entry
/// can collide with it.
pub const PLACEHOLDER_ID: &str = "\u{0}__empty__";

/// A disabled child that exists only to keep an empty or unread directory
/// classified as a folder.
fn placeholder_child() -> TreeItem {
    TreeItem::new(PLACEHOLDER_ID, "").disabled(true)
}

/// The tree id for a path: its absolute path string.
///
/// Using the path as the id is what lets a click resolve straight back to a
/// file, and what makes the id stable across rebuilds.
fn id_for(path: &Path) -> SharedString {
    SharedString::from(path.to_string_lossy().into_owned())
}

/// Fill in every open directory's children, reading the disk as needed.
///
/// Returns whether anything was read, so the caller knows to rebuild the tree.
/// This is the synchronous form used by tests and by the session restore, which
/// reads a handful of folders once; the interactive path reads one directory on
/// the background executor instead.
pub fn load_expanded(tree: &mut DirTree) -> bool {
    let mut changed = false;
    for dir in tree.expanded_without_children() {
        let (dir, children, truncated) = DirTree::read_listing(&dir);
        tree.insert_children(dir, children, truncated);
        changed = true;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lllearning-tree-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create the test directory");
        dir
    }

    #[test]
    fn an_unread_directory_is_still_a_folder() {
        // Risk R3: `TreeItem::is_folder()` is `!children.is_empty()`, so an
        // unread directory needs a placeholder or it cannot be expanded.
        let dir = temp_dir("folder-flag");
        std::fs::create_dir(dir.join("sub")).unwrap();
        std::fs::write(dir.join("file.txt"), b"x").unwrap();

        let mut tree = DirTree::new();
        tree.set_root(dir.clone()).unwrap();
        let items = tree.items();

        let sub = &items[0];
        assert_eq!(sub.label.as_ref(), "sub");
        assert!(
            sub.is_folder(),
            "an unread directory must still be expandable"
        );
        assert!(
            sub.children[0].is_disabled(),
            "the placeholder is not clickable"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_empty_directory_is_still_a_folder() {
        let dir = temp_dir("empty");
        std::fs::create_dir(dir.join("empty")).unwrap();

        let mut tree = DirTree::new();
        tree.set_root(dir.clone()).unwrap();
        tree.load_children(&dir.join("empty"));
        tree.expand(&dir.join("empty"));
        let items = tree.items();

        let empty = &items[0];
        assert!(
            empty.is_folder(),
            "an expanded empty directory stays a folder"
        );
        assert!(empty.children[0].is_disabled());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn children_load_only_when_a_directory_is_opened() {
        let dir = temp_dir("lazy");
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::write(dir.join("a/b/deep.txt"), b"x").unwrap();

        let mut tree = DirTree::new();
        tree.set_root(dir.clone()).unwrap();
        assert!(!tree.is_loaded(&dir.join("a")), "nothing is read up front");
        assert!(tree.is_loaded(&dir), "the root is read immediately");

        tree.expand(&dir.join("a"));
        assert!(load_expanded(&mut tree));
        assert!(tree.is_loaded(&dir.join("a")));
        assert!(!tree.is_loaded(&dir.join("a/b")), "b is still closed");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn expansion_state_survives_a_rebuild() {
        let dir = temp_dir("rebuild");
        std::fs::create_dir(dir.join("open")).unwrap();
        std::fs::write(dir.join("open/inner.txt"), b"x").unwrap();
        std::fs::create_dir(dir.join("shut")).unwrap();

        let mut tree = DirTree::new();
        tree.set_root(dir.clone()).unwrap();
        tree.expand(&dir.join("open"));
        load_expanded(&mut tree);

        let first = tree.items();
        let second = tree.items();
        let open = first
            .iter()
            .find(|item| item.label.as_ref() == "open")
            .unwrap();
        let open_again = second
            .iter()
            .find(|item| item.label.as_ref() == "open")
            .unwrap();

        assert!(open.is_expanded());
        assert!(
            open_again.is_expanded(),
            "the cache, not the tree, owns this"
        );
        assert!(open_again.is_folder());
        assert_eq!(
            open_again.children[0].label.as_ref(),
            "inner.txt",
            "a rebuild shows the same children"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reveal_opens_every_ancestor_below_the_root() {
        let dir = temp_dir("reveal");
        std::fs::create_dir_all(dir.join("a/b/c")).unwrap();
        let target = dir.join("a/b/c/file.txt");
        std::fs::write(&target, b"x").unwrap();

        let mut tree = DirTree::new();
        tree.set_root(dir.clone()).unwrap();
        tree.reveal(&target);

        assert!(tree.is_expanded(&dir.join("a")));
        assert!(tree.is_expanded(&dir.join("a/b")));
        assert!(tree.is_expanded(&dir.join("a/b/c")));
        assert_eq!(tree.selected(), Some(target.as_path()));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_clicked_directory_folds_and_unfolds_repeatedly() {
        // The tree toggles the row itself and reports the result; the cache
        // records it. Flipping the cache's own state instead made the two
        // disagree after the first click, so the second looked like it only
        // moved the selection.
        let dir = temp_dir("fold");
        std::fs::create_dir(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/one.txt"), b"x").unwrap();
        let sub = dir.join("sub");

        let mut tree = DirTree::new();
        tree.set_root(dir.clone()).unwrap();

        tree.set_expanded(&sub, true);
        load_expanded(&mut tree);
        assert!(tree.is_expanded(&sub));
        assert_eq!(tree.selected(), Some(sub.as_path()), "the click selects");

        tree.set_expanded(&sub, false);
        assert!(!tree.is_expanded(&sub), "the second click folds it");

        tree.set_expanded(&sub, true);
        assert!(tree.is_expanded(&sub), "and it opens again");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn invalidate_forces_a_reread_on_the_next_expand() {
        let dir = temp_dir("invalidate");
        std::fs::create_dir(dir.join("d")).unwrap();
        std::fs::write(dir.join("d/one.txt"), b"x").unwrap();

        let mut tree = DirTree::new();
        tree.set_root(dir.clone()).unwrap();
        tree.expand(&dir.join("d"));
        load_expanded(&mut tree);

        std::fs::write(dir.join("d/two.txt"), b"x").unwrap();
        tree.invalidate(&dir.join("d"));
        assert!(!tree.is_loaded(&dir.join("d")));

        tree.load_children(&dir.join("d"));
        let items = tree.items();
        let d = &items[0];
        let names: Vec<&str> = d.children.iter().map(|item| item.label.as_ref()).collect();
        assert_eq!(names, vec!["one.txt", "two.txt"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_root_leaves_an_empty_tree() {
        let tree = DirTree::new();
        assert!(tree.items().is_empty());
        assert!(tree.root().is_none());
    }

    #[test]
    fn set_root_on_a_missing_directory_reports_the_error() {
        let mut tree = DirTree::new();
        let missing = std::env::temp_dir().join("lllearning-does-not-exist-xyz");
        assert!(tree.set_root(missing).is_err());
        assert!(
            tree.root().is_none(),
            "a failed root leaves the old state clean"
        );
    }
}
