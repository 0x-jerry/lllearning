//! The workspace: which folder is open, its tree, and its change notifications.
//!
//! This is one of the three seams the shell knows about. It owns the folder
//! root, the lazily-read directory cache ([`tree::DirTree`]), the `TreeState`
//! the explorer renders, and the filesystem watcher. Nothing above it reaches
//! into the filesystem directly.

pub mod fs_util;
pub mod tree;
pub mod watcher;

use std::path::{Path, PathBuf};

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, PathPromptOptions, SharedString, Task,
    WeakEntity, Window, base::TreeState,
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

        if let Err(error) = self.tree.set_root(canonical.clone()) {
            cx.emit(WorkspaceEvent::Error(format!(
                "{} could not be read: {error}",
                canonical.display()
            )));
            return;
        }

        self.root = Some(canonical.clone());
        self.restart_watcher(&canonical, window, cx);
        self.rebuild_tree_state(cx);
        cx.emit(WorkspaceEvent::RootChanged);
        cx.notify();
    }

    /// Forget the open folder, its tree and its watcher.
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
    pub fn toggle_dir(&mut self, dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let opened = self.tree.toggle_expanded(&dir);
        // The open state must show immediately or the click looks ignored.
        self.rebuild_tree_state(cx);

        if opened && !self.tree.is_loaded(&dir) {
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
    pub fn reveal(&mut self, path: &Path, cx: &mut Context<Self>) {
        self.tree.reveal(path);
        self.rebuild_tree_state(cx);
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
    fn restart_watcher(&mut self, root: &Path, window: &Window, cx: &mut Context<Self>) {
        // Dropping these stops the old watch and its drain task.
        self.watcher = None;
        self.watcher_task = None;

        let root = root.to_path_buf();
        match watcher::FileWatcher::start(root.clone()) {
            Ok((file_watcher, receiver)) => {
                self.watcher = Some(file_watcher);
                self.watcher_task = Some(cx.spawn_in(window, async move |this, cx| {
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
            }
            Err(error) => {
                cx.emit(WorkspaceEvent::Error(format!(
                    "watching {} failed: {error}",
                    root.display()
                )));
            }
        }
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
pub fn is_inside(root: Option<&Path>, path: &Path) -> bool {
    root.is_some_and(|root| path.starts_with(root) || fs_util::same_path(root, path))
}

/// The label for a workspace root: its folder name.
pub fn root_label(root: &Path) -> String {
    fs_util::file_name_of(root)
}

/// Keep a weak handle type alias readable at call sites that hold several.
pub type WorkspaceHandle = WeakEntity<Workspace>;

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
}
