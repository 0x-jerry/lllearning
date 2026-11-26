//! The workspace: which folder is open, its tree, and its change notifications.
//!
//! This is one of the three seams the shell knows about. It owns the folder
//! root, the lazily-read directory cache ([`tree::DirTree`]), the `TreeState`
//! the explorer renders, and the filesystem watcher. Nothing above it reaches
//! into the filesystem directly.
//!
//! # Why some members are `pub` but only exercised by tests
//!
//! The workspace's readers — [`Workspace::tree`], the path predicates, the
//! synchronous [`tree::load_expanded`] — are part of the seam's contract and are
//! asserted by unit tests rather than by every call site in the binary. The
//! dead-code lint cannot see a test-only consumer, so it is allowed on those
//! items rather than trimming the API to whatever the current UI happens to touch.

pub mod fs_util;
pub mod tree;
pub mod watcher;

use std::path::{Path, PathBuf};

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, PathPromptOptions, SharedString, Task,
    Window, base::TreeState,
};

use tree::DirTree;

/// Something the workspace changed that the rest of the app cares about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceEvent {
    /// A different folder is open, or none is.
    RootChanged,
    /// A directory's listing changed, so the tree needs rebuilding.
    TreeChanged(PathBuf),
    /// Files changed on disk outside the app.
    DiskChanged(Vec<PathBuf>),
    /// The workspace could not do what was asked, with a message for the user.
    Error(String),
}

/// The workspace folder, its tree, and its watcher.
pub struct Workspace {
    /// The open folder. `None` until one is picked, or after the user clears it.
    root: Option<PathBuf>,
    tree: DirTree,
    /// The tree the explorer renders. Owned here so a rebuild from any caller
    /// reaches the same widget.
    tree_state: Entity<TreeState>,
    /// The recursive watcher, rebuilt whenever the root changes.
    watcher: Option<watcher::FileWatcher>,
    /// The task draining the watcher's channel into events. Held so dropping
    /// the workspace stops the drain.
    watcher_task: Option<Task<()>>,
}

impl EventEmitter<WorkspaceEvent> for Workspace {}

impl Workspace {
    /// A workspace with no folder open.
    pub fn new(cx: &mut App) -> Entity<Self> {
        cx.new(|cx| {
            let tree_state = cx.new(|cx| TreeState::new(cx));
            Self {
                root: None,
                tree: DirTree::new(),
                tree_state,
                watcher: None,
                watcher_task: None,
            }
        })
    }

    /// The open folder.
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// The tree the explorer renders.
    pub fn tree_state(&self) -> &Entity<TreeState> {
        &self.tree_state
    }

    /// The directory cache, for the explorer to render icons and expansion from.
    #[allow(dead_code)]
    pub fn tree(&self) -> &DirTree {
        &self.tree
    }

    /// Whether a watcher is running, for the status bar.
    pub fn is_watching(&self) -> bool {
        self.watcher.is_some()
    }

    /// Open `path` as the workspace folder.
    ///
    /// Canonicalises first, so every path derived from the root is absolute and
    /// free of `..`. A folder that no longer exists is reported rather than
    /// left as a broken root.
    ///
    /// The folder's listing and its watcher are read on the background
    /// executor (risk R8). The root is published immediately, so the panes that
    /// read it are not held up, and the tree fills in when the listing lands.
    pub fn set_root(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let canonical = match std::fs::canonicalize(&path) {
            Ok(canonical) => canonical,
            Err(error) => {
                cx.emit(WorkspaceEvent::Error(format!(
                    "{} could not be opened: {error}",
                    path.display()
                )));
                return;
            }
        };

        // `read_dir` used to happen here, to prove the folder was readable
        // before it was published. It is the same `read_dir` the background read
        // below performs, so it happens once, there — but a root that cannot be
        // read at all is still refused up front, because publishing it would
        // leave a folder in the tree that can never be expanded.
        if let Err(error) = std::fs::read_dir(&canonical) {
            cx.emit(WorkspaceEvent::Error(format!(
                "{} could not be read: {error}",
                canonical.display()
            )));
            return;
        }

        self.tree.set_root_unread(canonical.clone());
        self.root = Some(canonical.clone());
        self.restart_watcher(&canonical, window, cx);
        self.rebuild_tree_state(cx);
        cx.emit(WorkspaceEvent::RootChanged);
        cx.notify();

        self.read_root_listing(canonical, window, cx);
    }

    /// Read the new root's children off the UI thread and install them.
    fn read_root_listing(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        let dir = root.clone();
        cx.spawn_in(window, async move |_, cx| {
            let listing = cx
                .background_spawn(async move { DirTree::read_listing(&dir) })
                .await;
            let _ = this.update_in(cx, |this, _, cx| {
                let (dir, children, truncated) = listing;
                // An empty listing for a directory that cannot be read is how
                // `read_listing` reports a failure, so the root is only accepted
                // when it produced something or is genuinely empty on disk.
                if children.is_empty() && !dir.is_dir() {
                    cx.emit(WorkspaceEvent::Error(format!(
                        "{} could not be read",
                        dir.display()
                    )));
                    return;
                }
                this.tree.insert_children(dir.clone(), children, truncated);
                this.rebuild_tree_state(cx);
                cx.emit(WorkspaceEvent::TreeChanged(dir));
                cx.notify();
            });
        })
        .detach();
    }

    /// Forget the open folder, its tree and its watcher.
    #[allow(dead_code)]
    pub fn clear_root(&mut self, cx: &mut Context<Self>) {
        self.tree.reset();
        self.root = None;
        self.watcher = None;
        self.watcher_task = None;
        let tree_state = self.tree_state.clone();
        tree_state.update(cx, |state, cx| state.set_items(Vec::new(), cx));
        cx.emit(WorkspaceEvent::RootChanged);
        cx.notify();
    }

    /// Ask the user for a folder and open it.
    ///
    /// The picker is a platform modal, so the answer arrives asynchronously;
    /// the workspace entity is kept alive across the wait by the weak handle.
    pub fn pick_root(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(SharedString::from("Choose a workspace folder")),
        });

        cx.spawn_in(window, async move |this, cx| {
            let picked = match receiver.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                // Cancelled: nothing to do, and nothing to report.
                Ok(Ok(None)) => None,
                Ok(Err(error)) => {
                    let message = format!("the folder picker failed: {error}");
                    let _ = this.update(cx, |_, cx| cx.emit(WorkspaceEvent::Error(message)));
                    return;
                }
                Err(error) => {
                    let message = format!("the folder picker closed unexpectedly: {error}");
                    let _ = this.update(cx, |_, cx| cx.emit(WorkspaceEvent::Error(message)));
                    return;
                }
            };

            if let Some(path) = picked {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.set_root(path, window, cx);
                });
            }
        })
        .detach();
    }

    /// Open or close a directory the user clicked, reading its children if it
    /// was opened for the first time.
    ///
    /// The tree has already toggled the row and reported which way it went;
    /// this records that in the cache and reads the directory if it is being
    /// opened for the first time.
    pub fn toggle_dir(
        &mut self,
        dir: PathBuf,
        expanded: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tree.set_expanded(&dir, expanded);
        // The open state must show immediately or the click looks ignored.
        self.rebuild_tree_state(cx);

        if expanded && !self.tree.is_loaded(&dir) {
            let this = cx.weak_entity();
            let dir_for_read = dir.clone();
            cx.spawn_in(window, async move |_, cx| {
                let listing = cx
                    .background_spawn(async move { DirTree::read_listing(&dir_for_read) })
                    .await;
                let _ = this.update_in(cx, |this, _, cx| {
                    let (dir, children, truncated) = listing;
                    this.tree.insert_children(dir.clone(), children, truncated);
                    this.rebuild_tree_state(cx);
                    cx.emit(WorkspaceEvent::TreeChanged(dir));
                    cx.notify();
                });
            })
            .detach();
        }
        cx.notify();
    }

    /// Re-read every open directory from disk.
    pub fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut dirs = self.tree.open_dirs();
        if let Some(root) = &self.root
            && !dirs.iter().any(|dir| dir == root)
        {
            dirs.push(root.clone());
        }
        if dirs.is_empty() {
            return;
        }

        let this = cx.weak_entity();
        cx.spawn_in(window, async move |_, cx| {
            let listings = cx
                .background_spawn(async move {
                    dirs.into_iter()
                        .map(|dir| DirTree::read_listing(&dir))
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.tree.invalidate_all();
                this.tree.insert_listings(listings);
                this.rebuild_tree_state(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Re-read the directories the watcher reported, keeping expansion state.
    fn reread_dirs(&mut self, dirs: Vec<PathBuf>) {
        for dir in dirs {
            let (dir, children, truncated) = DirTree::read_listing(&dir);
            self.tree.insert_children(dir, children, truncated);
        }
    }

    /// Highlight `path` and open the folders leading to it.
    ///
    /// The folders are read on the background executor, like every other
    /// listing (risk R8): a reveal into a directory that was never expanded has
    /// to read it, and doing that inline would freeze the window on a network
    /// share. The selection and the scroll happen once those listings land, so
    /// the tree never shows a highlight on a row it has not built yet.
    pub fn reveal(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        self.tree.reveal(path);
        self.rebuild_tree_state(cx);

        let missing = self.tree.expanded_without_children();
        if missing.is_empty() {
            self.scroll_to_selection(cx);
            return;
        }

        let target = path.to_path_buf();
        let this = cx.weak_entity();
        cx.spawn_in(window, async move |_, cx| {
            let listings = cx
                .background_spawn(async move {
                    missing
                        .into_iter()
                        .map(|dir| DirTree::read_listing(&dir))
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.tree.insert_listings(listings);
                this.rebuild_tree_state(cx);
                this.scroll_to_selection(cx);
                cx.emit(WorkspaceEvent::TreeChanged(target));
                cx.notify();
            });
        })
        .detach();
    }

    /// Scroll the tree so the selected row is on screen.
    ///
    /// Only a reveal does this: a refresh or a watcher rebuild keeps the scroll
    /// where the user left it, or the tree would jump under them every time a
    /// file changed on disk.
    fn scroll_to_selection(&mut self, cx: &mut Context<Self>) {
        let selected = self
            .tree
            .selected()
            .map(|path| SharedString::from(path.to_string_lossy().into_owned()));
        let tree_state = self.tree_state.clone();
        tree_state.update(cx, |state, cx| {
            if let Some(ix) = selected.and_then(|id| state.index_of(&id)) {
                state.scroll_to_item(ix, gpui::ScrollStrategy::Center);
                cx.notify();
            }
        });
    }

    /// Rebuild the `TreeState` from the cache, restoring the selection.
    fn rebuild_tree_state(&mut self, cx: &mut Context<Self>) {
        let items = self.tree.items();
        let selected = self
            .tree
            .selected()
            .map(|path| SharedString::from(path.to_string_lossy().into_owned()));

        let tree_state = self.tree_state.clone();
        tree_state.update(cx, |state, cx| {
            state.set_items(items, cx);
            // `set_items` clears the selection (risk R5), so it is restored here
            // from the cache rather than left to the click handler.
            if let Some(id) = selected
                && let Some(ix) = state.index_of(&id)
            {
                state.set_selected_index(Some(ix), cx);
            }
        });
    }

    /// Start watching `root`, replacing any previous watcher.
    ///
    /// The watch is registered on the background executor: `notify` walks the
    /// tree to install a recursive watch, which is a filesystem call like any
    /// other and does not belong on the UI thread (risk R8). The status bar
    /// reads `is_watching`, so the watcher is only published once it exists.
    fn restart_watcher(&mut self, root: &Path, window: &Window, cx: &mut Context<Self>) {
        // Dropping these stops the old watch and its drain task.
        self.watcher = None;
        self.watcher_task = None;

        let root = root.to_path_buf();
        let this = cx.weak_entity();
        cx.spawn_in(window, async move |_, cx| {
            let watching = cx
                .background_spawn({
                    let root = root.clone();
                    async move { watcher::FileWatcher::start(root) }
                })
                .await;

            let _ = this.update_in(cx, |this, window, cx| match watching {
                Ok((file_watcher, receiver)) => {
                    this.watcher = Some(file_watcher);
                    this.watcher_task = Some(cx.spawn_in(window, async move |this, cx| {
                        while let Ok(paths) = receiver.recv().await {
                            if paths.is_empty() {
                                continue;
                            }
                            let done = this.update_in(cx, |this, _, cx| {
                                this.on_disk_changes(&paths, cx);
                            });
                            if done.is_err() {
                                break;
                            }
                        }
                    }));
                    cx.notify();
                }
                Err(error) => {
                    cx.emit(WorkspaceEvent::Error(format!(
                        "watching {} failed: {error}",
                        root.display()
                    )));
                }
            });
        })
        .detach();
    }

    /// Fold a batch of watcher paths into cache updates and app events.
    fn on_disk_changes(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };

        // The tree only cares about directories whose listing is cached and
        // shown: an unread directory is read correctly when it is expanded, and
        // a closed one is re-read on expand only if invalidated.
        let mut changed_dirs = Vec::new();
        for path in paths {
            if fs_util::is_ignored_path(path, &root) {
                continue;
            }
            // A change to `dir/x` changes `dir`'s listing.
            let dir = if path.is_dir() {
                path.clone()
            } else {
                path.parent().unwrap_or(path).to_path_buf()
            };
            // A closed directory's cache must still be dropped, or reopening it
            // would show a stale listing; only the rebuild is limited to the
            // directories actually on screen.
            self.tree.invalidate(&dir);
            if self.tree.is_expanded(&dir) {
                changed_dirs.push(dir);
            }
        }
        changed_dirs.sort();
        changed_dirs.dedup();

        if !changed_dirs.is_empty() {
            self.reread_dirs(changed_dirs.clone());
            self.rebuild_tree_state(cx);
            cx.emit(WorkspaceEvent::TreeChanged(changed_dirs[0].clone()));
        }
        cx.emit(WorkspaceEvent::DiskChanged(paths.to_vec()));
        cx.notify();
    }
}

/// Whether `path` is inside the workspace root.
#[allow(dead_code)]
pub fn is_inside(root: Option<&Path>, path: &Path) -> bool {
    root.is_some_and(|root| path.starts_with(root) || fs_util::same_path(root, path))
}

/// The label for a workspace root: its folder name.
#[allow(dead_code)]
pub fn root_label(root: &Path) -> String {
    fs_util::file_name_of(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_inside_the_root_is_reported() {
        let root = Path::new("/work");
        assert!(is_inside(Some(root), Path::new("/work/src/main.rs")));
        assert!(is_inside(Some(root), Path::new("/work")));
        assert!(!is_inside(Some(root), Path::new("/elsewhere/x")));
        assert!(!is_inside(None, Path::new("/work/x")));
    }

    #[test]
    fn the_root_label_is_the_folder_name() {
        assert_eq!(root_label(Path::new("/a/b/lllearning")), "lllearning");
    }

    /// Build an empty throwaway folder.
    ///
    /// `tag` must differ between calls in one test: the thread id is the same,
    /// so the tag is what keeps two folders from sharing a path.
    fn temp_workspace(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lllearning-ws-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create the folder");
        dir
    }

    /// Changing the folder replaces the tree and drops the old one.
    ///
    /// The folder switch is the half of the flow that does not need a platform
    /// folder picker: `set_root` is what the picker's reply calls.
    #[test]
    fn changing_the_root_replaces_the_tree() {
        let first = temp_workspace("first");
        let second = temp_workspace("second");
        std::fs::write(first.join("only-in-first.txt"), b"x").expect("write the first marker");
        std::fs::write(second.join("only-in-second.txt"), b"y").expect("write the second marker");

        let mut tree = crate::workspace::tree::DirTree::new();
        tree.set_root(first.clone()).expect("first root");
        assert_eq!(tree.root(), Some(first.as_path()));
        let first_names: Vec<String> = tree
            .items()
            .iter()
            .map(|item| item.label.to_string())
            .collect();
        assert_eq!(first_names, vec!["only-in-first.txt"]);

        tree.set_root(second.clone()).expect("second root");
        assert_eq!(tree.root(), Some(second.as_path()));

        let names: Vec<String> = tree
            .items()
            .iter()
            .map(|item| item.label.to_string())
            .collect();
        assert_eq!(
            names,
            vec!["only-in-second.txt"],
            "the new folder's listing replaces the old one entirely"
        );

        std::fs::remove_dir_all(&first).ok();
        std::fs::remove_dir_all(&second).ok();
    }

    /// A root that no longer exists is reported, not silently accepted.
    #[test]
    fn a_missing_root_is_refused() {
        let mut tree = crate::workspace::tree::DirTree::new();
        let missing = std::env::temp_dir().join("lllearning-not-a-folder-xyz");
        assert!(tree.set_root(missing).is_err());
        assert_eq!(tree.root(), None, "a failed root leaves nothing behind");
    }
}
