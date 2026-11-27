//! The explorer's lazy directory cache, and the translation from it to
//! `TreeItem`s. Nothing is read until a directory is expanded.
//!
//! Expansion state lives here, not in `TreeItem`: a rebuild consumes the tree, so
//! whatever the cache hands out already carries the right flags.

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
    /// The highlighted entry, so a rebuild can restore it.
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
    #[allow(dead_code)]
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// The highlighted entry, if any.
    pub fn selected(&self) -> Option<&Path> {
        self.selected.as_deref()
    }

    /// Record the highlighted entry without touching the disk.
    #[allow(dead_code)]
    pub fn set_selected(&mut self, path: Option<PathBuf>) {
        self.selected = path;
    }

    /// Record what a click did. The `TreeState` toggles the row first, so the
    /// cache is told the new state rather than flipping its own.
    pub fn set_expanded(&mut self, dir: &Path, expanded: bool) {
        if expanded {
            self.expanded.insert(dir.to_path_buf());
        } else {
            self.expanded.remove(dir);
        }
        self.selected = Some(dir.to_path_buf());
    }

    /// Whether a directory's listing was capped.
    #[allow(dead_code)]
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

    /// Start a new workspace at `root` and read its children here. The UI uses
    /// [`Self::set_root_unread`] and reads on the background executor instead.
    #[allow(dead_code)]
    pub fn set_root(&mut self, root: PathBuf) -> std::io::Result<()> {
        self.set_root_unread(root.clone());
        match read_dir_sorted(&root) {
            Ok((children, truncated)) => {
                self.loaded.insert(root.clone(), LoadedDir { children });
                if truncated {
                    self.truncated.insert(root);
                }
                Ok(())
            }
            // A root that cannot be read is not a root.
            Err(error) => {
                self.reset();
                Err(error)
            }
        }
    }

    /// Start a new workspace at `root` without reading it. The caller installs
    /// the listing with [`Self::insert_children`].
    pub fn set_root_unread(&mut self, root: PathBuf) {
        self.reset();
        self.root = Some(root.clone());
        self.loaded.insert(root.clone(), LoadedDir::default());
        self.expanded.insert(root);
    }

    /// Drop the root and every cached listing.
    pub fn reset(&mut self) {
        self.root = None;
        self.loaded.clear();
        self.expanded.clear();
        self.selected = None;
        self.truncated.clear();
    }

    /// Read a directory's children into the cache, unless already there. An
    /// unreadable directory is cached as empty so a second expand does not retry.
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
    pub fn invalidate(&mut self, dir: &Path) {
        self.loaded.remove(dir);
        self.truncated.remove(dir);
    }

    /// Re-read a cached directory, keeping the expansion state so the tree does
    /// not flicker closed.
    #[allow(dead_code)]
    pub fn refresh(&mut self, dir: &Path) -> bool {
        if !self.loaded.contains_key(dir) {
            return false;
        }
        self.invalidate(dir);
        self.load_children(dir)
    }

    /// Forget every cached listing, keeping the root and what is open.
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

    /// Read a directory on the current thread for the caller to install. Static
    /// so a background task can call it without holding the tree.
    pub fn read_listing(dir: &Path) -> (PathBuf, Vec<DirEntryInfo>, bool) {
        match read_dir_sorted(dir) {
            Ok((children, truncated)) => (dir.to_path_buf(), children, truncated),
            Err(_) => (dir.to_path_buf(), Vec::new(), false),
        }
    }

    /// Mark a directory open without toggling, for "reveal" and for restore.
    #[allow(dead_code)]
    pub fn expand(&mut self, dir: &Path) {
        self.expanded.insert(dir.to_path_buf());
    }

    /// Open every ancestor of `path` below the root, and select it. The caller
    /// still has to load the children it wants visible.
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
    pub fn open_dirs(&self) -> Vec<PathBuf> {
        let mut dirs: Vec<PathBuf> = self.expanded.iter().cloned().collect();
        dirs.sort();
        dirs
    }

    /// Install a listing that was read elsewhere, usually on the background
    /// executor.
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
    pub fn expanded_without_children(&self) -> Vec<PathBuf> {
        self.expanded
            .iter()
            .filter(|dir| !self.loaded.contains_key(*dir))
            .cloned()
            .collect()
    }

    /// The tree, ready for `TreeState::set_items`.
    ///
    /// The root's *children* are the top level: the folder is already named in
    /// the toolbar, and a row around everything would cost a level of
    /// indentation. A directory that was never read appears as a leaf.
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
            // A closed directory keeps a placeholder child so `is_folder()` stays
            // true and the row is expandable.
            return item.child(placeholder_child());
        }

        match self.loaded.get(path) {
            Some(loaded) if !loaded.children.is_empty() => {
                let children = self.children_of(path);
                item.children(children).expanded(true)
            }
            // An open directory with nothing in it still needs the placeholder.
            _ => item.child(placeholder_child()).expanded(true),
        }
    }

    /// Whether `path` is a directory according to the cache.
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

/// The tree id for a path: its absolute path string, which resolves back to a
/// file and stays stable across rebuilds.
fn id_for(path: &Path) -> SharedString {
    SharedString::from(path.to_string_lossy().into_owned())
}

/// Fill in every open directory's children, reading the disk as needed. The
/// synchronous form: tests and the session restore, which reads a handful of
/// folders once.
#[allow(dead_code)]
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
        // `TreeItem::is_folder()` is `!children.is_empty()`.
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
