//! The application shell.
//!
//! The shell knows three seams and nothing else: [`PaneRegistry`] for the side
//! slots, [`DocumentRegistry`] for the center, and [`Workspace`] for the folder.
//! It has no idea what a file, an editor or a webview is — which is what makes
//! the panes and the viewers replaceable without touching this file.
//!
//! # Why some accessors are `pub` but unused by the binary
//!
//! [`AppShell::dock`], [`AppShell::active_panes`], [`AppShell::active_tab`] and
//! [`AppShell::close_document`] are the shell's own seam: a test drives the dock
//! and the close flow through them, and a later feature (a keyboard shortcut, a
//! menu) would too. The dead-code lint cannot see a test-only consumer, so it is
//! relaxed here rather than hiding the shell's state behind an incomplete API.
#![allow(dead_code)]

use std::{collections::HashMap, path::PathBuf};

use gpui::base::dock::{DockArea, DockEvent, DockLayout, DockPlacement, PanelId};
use gpui::component::{ActiveTheme as _, button::ButtonVariants as _};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Window, div, px,
};
use gpui_rsx::rsx;

use crate::{
    app::{activity_bar::ActivityBar, skin::ShellSkin, status_bar, title_bar},
    documents::DocumentPanel,
    panes::{self, PaneSlot, ShellHandles},
    state::SessionState,
    workspace::{Workspace, WorkspaceEvent},
};

/// The application shell.
pub struct AppShell {
    focus_handle: FocusHandle,
    /// The dock area: left dock, center, right dock.
    dock: Entity<DockArea>,
    /// The folder, its tree and its watcher.
    workspace: Entity<Workspace>,
    /// Which pane each side dock is showing, by provider id.
    active_panes: HashMap<PaneSlot, String>,
    /// Every document panel currently open, so the shell can drive them.
    documents: Vec<Entity<DocumentPanel>>,
    /// The document the center is showing, if any.
    active_document: Option<Entity<DocumentPanel>>,
    /// A URL the browser pane should load next, put here by a viewer that cannot
    /// play a file itself.
    browser_request: Option<String>,
    /// The pending debounced session write, held so a newer change can reuse it.
    save_task: Option<gpui::Task<()>>,
    /// Dropped with the shell, so the subscriptions do not outlive it.
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<()> for AppShell {}

impl Focusable for AppShell {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl AppShell {
    /// Build the shell, restoring `session` if it is present.
    ///
    /// The window's build closure hands out `&mut App`, so the entity is created
    /// here and its construction work runs inside `cx.new`'s own context.
    pub fn new(session: SessionState, window: &mut Window, cx: &mut App) -> Entity<Self> {
        let workspace = Workspace::new(cx);
        let (dock, skin) = ShellSkin::dock_area("workspace", Some(1), window, cx);

        let shell = cx.new(|cx| {
            let mut shell = Self {
                focus_handle: cx.focus_handle(),
                dock: dock.clone(),
                workspace: workspace.clone(),
                active_panes: HashMap::new(),
                documents: Vec::new(),
                active_document: None,
                browser_request: None,
                save_task: None,
                _subscriptions: Vec::new(),
            };
            shell.subscribe_all(window, cx);
            shell.restore_session(session, window, cx);
            shell
        });

        // A panel builder runs inside the dock's registry, which carries only
        // the dock area and the persisted panel state; the workspace and the
        // shell are published here so a rebuilt pane can reach them.
        ShellHandles::set(cx, workspace, shell.downgrade());

        // The skin asks the shell before it closes a tab, because only the
        // shell knows whether the panel behind it has unsaved work.
        let gate_shell = shell.downgrade();
        skin.set_close_request(std::rc::Rc::new(move |panel_id, window, cx| {
            gate_shell
                .update(cx, |shell, cx| shell.allow_close(panel_id, window, cx))
                .unwrap_or(true)
        }));

        // Closing the window hides it to the tray rather than quitting; the
        // tray's Quit is what ends the app.
        crate::tray::keep_window_on_close(window, cx);

        shell
    }

    /// Register the quit-time session save and dirty guard.
    ///
    /// Called once, after the window exists. The save is unconditional — a
    /// session file is a convenience, not user data — but unsaved documents are
    /// reported on stderr before the app goes, because there is no window left
    /// to prompt in by the time this runs.
    pub fn install_quit_hook(&mut self, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        let subscription = cx.on_app_quit(move |this_shell, cx| {
            if let Some(shell) = this.upgrade() {
                let shell = shell.read(cx);
                if any_dirty(shell, cx) {
                    eprintln!(
                        "quitting with {} unsaved document(s)",
                        shell
                            .documents()
                            .iter()
                            .filter(|document| document.read(cx).is_dirty(cx))
                            .count()
                    );
                }
                crate::state::save(&shell.dump_session(cx));
            }
            let _ = this_shell;
            async {}
        });
        self._subscriptions.push(subscription);
    }

    /// Whether the dock may close `panel_id` right now.
    ///
    /// A clean document closes at once. A dirty one is not closed: the shell
    /// opens its Save / Discard / Cancel prompt instead, and the prompt does the
    /// closing when the user answers.
    fn allow_close(
        &mut self,
        panel_id: PanelId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let document = self
            .documents
            .iter()
            .find(|document| PanelId::from(document.entity_id()) == panel_id)
            .cloned();

        match document {
            Some(document) if document.read(cx).is_dirty(cx) => {
                self.prompt_save_then_close(document, window, cx);
                false
            }
            // A side pane, or a clean document: the dock closes it.
            _ => true,
        }
    }

    /// The workspace folder.
    pub fn workspace(&self) -> &Entity<Workspace> {
        &self.workspace
    }

    /// The dock area, for tests.
    pub fn dock(&self) -> &Entity<DockArea> {
        &self.dock
    }

    /// The documents currently open.
    pub fn documents(&self) -> &[Entity<DocumentPanel>] {
        &self.documents
    }

    /// The document the center is showing.
    pub fn active_document(&self) -> Option<&Entity<DocumentPanel>> {
        self.active_document.as_ref()
    }

    /// Which provider each side dock is showing.
    pub fn active_panes(&self) -> &HashMap<PaneSlot, String> {
        &self.active_panes
    }

    fn subscribe_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The dock tells us when the layout changed, which is when the active
        // tab may have moved.
        let dock_subscription = cx.subscribe_in(
            &self.dock,
            window,
            |this: &mut Self, _, event: &DockEvent, window, cx| {
                if matches!(event, DockEvent::LayoutChanged) {
                    this.sync_active_document(cx);
                    // A moved divider, a new tab or a closed tab all land here.
                    this.schedule_session_save(window, cx);
                    cx.notify();
                }
            },
        );

        let workspace_subscription = cx.subscribe_in(
            &self.workspace,
            window,
            |this: &mut Self, _, event: &WorkspaceEvent, window, cx| {
                this.on_workspace_event(event, window, cx);
            },
        );

        self._subscriptions = vec![dock_subscription, workspace_subscription];
    }

    /// React to a workspace event.
    fn on_workspace_event(
        &mut self,
        event: &WorkspaceEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            WorkspaceEvent::RootChanged => {
                // A different folder means the old tabs no longer belong to the
                // workspace, so they are closed rather than left pointing at
                // files that are not in the tree.
                self.close_all_documents(window, cx);
                self.mount_side_panes(window, cx);
                self.schedule_session_save(window, cx);
            }
            WorkspaceEvent::TreeChanged(_) => {}
            WorkspaceEvent::DiskChanged(paths) => {
                self.sync_disk_changes(paths, window, cx);
            }
            WorkspaceEvent::Error(message) => {
                eprintln!("workspace: {message}");
            }
        }
        cx.notify();
    }

    /// Bring each open document in line with what changed on disk.
    ///
    /// A clean buffer is reloaded silently, because the user had no work in it.
    /// A dirty buffer is only marked: overwriting unsaved edits with the disk
    /// version would lose them (risk R11), so the panel shows a banner and the
    /// user decides.
    fn sync_disk_changes(
        &mut self,
        paths: &[PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for path in paths {
            let matching: Vec<Entity<DocumentPanel>> = self
                .documents
                .iter()
                .filter(|document| {
                    crate::workspace::fs_util::same_path(document.read(cx).path(), path)
                })
                .cloned()
                .collect();

            for document in matching {
                let dirty = document.read(cx).is_dirty(cx);
                let controller = document.read(cx).controller().clone();
                if dirty {
                    controller.mark_stale(cx);
                } else {
                    controller.reload_from_disk(window, cx);
                }
                document.update(cx, |_, cx| cx.notify());
            }
        }
    }

    /// Install the default layout when there is nothing to restore.
    fn default_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mount_side_panes(window, cx);

        self.dock.update(cx, |dock, cx| {
            dock.set_center(DockLayout::tabs(), window, cx)
        });
    }

    /// Build and mount one panel per registered side provider.
    ///
    /// This is what makes a new pane appear with no shell edit: the providers
    /// come from the registry, and each one builds its own panel.
    fn mount_side_panes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for slot in PaneSlot::ALL {
            let providers = panes::panes_for(cx, slot);
            if providers.is_empty() {
                continue;
            }

            let mut layout = DockLayout::tabs();
            for provider in &providers {
                let panel = provider.build(
                    panes::PaneContext {
                        workspace: self.workspace.clone(),
                        shell: cx.weak_entity(),
                        saved: None,
                    },
                    window,
                    cx,
                );
                layout = layout.panel_view(panel, cx);
            }

            let size = match slot {
                PaneSlot::Left => px(260.),
                PaneSlot::Right => px(420.),
            };
            self.dock.update(cx, |dock, cx| {
                dock.set_dock(slot.placement(), layout, window, cx);
                dock.set_dock_size(slot.placement(), size, window, cx);
            });

            // The first provider is the one a fresh session shows.
            let first = providers[0].id().to_owned();
            self.active_panes.insert(slot, first);
        }
    }

    /// Restore the saved session, falling back to the default layout.
    fn restore_session(
        &mut self,
        session: SessionState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The workspace first: the dock's panel builders read it.
        if let Some(root) = session.workspace_root.clone()
            && root.exists()
        {
            self.workspace
                .update(cx, |workspace, cx| workspace.set_root(root, window, cx));
        }

        match session.dock.clone() {
            Some(state) => {
                let loaded = self
                    .dock
                    .update(cx, |dock, cx| dock.load(state, window, cx));
                if loaded.is_err() {
                    // A layout this build cannot rebuild is worse than none.
                    eprintln!("the saved layout could not be restored; using the default");
                    self.default_layout(window, cx);
                } else {
                    self.active_panes = session
                        .active_left
                        .clone()
                        .map(|id| (PaneSlot::Left, id))
                        .into_iter()
                        .chain(session.active_right.clone().map(|id| (PaneSlot::Right, id)))
                        .collect();
                }
            }
            None => self.default_layout(window, cx),
        }
    }

    /// Open a file in the center.
    pub fn open_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let canonical = std::fs::canonicalize(&path).unwrap_or(path);

        // A tab for this file already exists: select it rather than duplicate.
        let existing = self
            .documents
            .iter()
            .find(|document| {
                crate::workspace::fs_util::same_path(document.read(cx).path(), &canonical)
            })
            .cloned();
        if let Some(existing) = existing {
            self.select_document(&existing, window, cx);
            return;
        }

        let opened = DocumentPanel::open(
            canonical.clone(),
            cx.weak_entity(),
            self.workspace.clone(),
            window,
            cx,
        );
        let Some(panel) = opened else {
            eprintln!("no viewer can show {}", canonical.display());
            return;
        };

        let handle = gpui::component::dock::panel_handle(panel.clone());
        self.dock.update(cx, |dock, cx| {
            dock.add_panel_view(handle, DockPlacement::Center, None, window, cx);
        });

        self.documents.push(panel.clone());
        self.active_document = Some(panel.clone());
        let controller = panel.read(cx).controller().clone();
        controller.focus(window, cx);
        self.workspace
            .update(cx, |workspace, cx| workspace.reveal(&canonical, cx));
        cx.notify();
    }

    /// Show an already-open document.
    fn select_document(
        &mut self,
        document: &Entity<DocumentPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panel_id = PanelId::from(document.entity_id());
        self.dock.update(cx, |dock, cx| {
            dock.select_panel(panel_id, window, cx);
        });
        // Selecting a tab is also focusing it: the editor should take the
        // keyboard, or the user's next keystroke goes to the last thing that had
        // it.
        let controller = document.read(cx).controller().clone();
        controller.focus(window, cx);
        self.active_document = Some(document.clone());
        cx.notify();
    }

    /// Close one document, prompting if it has unsaved changes.
    ///
    /// This is the path a keyboard shortcut or a menu uses. The tab bar's own
    /// close button goes through [`Self::allow_close`], which calls the same
    /// prompt.
    pub fn close_document(
        &mut self,
        document: Entity<DocumentPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if document.read(cx).is_dirty(cx) {
            self.prompt_save_then_close(document, window, cx);
            return;
        }
        self.remove_document(&document, window, cx);
    }

    /// The tab the dock is showing, if it is a document.
    pub fn active_tab(&self, cx: &App) -> Option<PanelId> {
        self.dock
            .read(cx)
            .layout(DockPlacement::Center)
            .and_then(active_panel_in_tree)
    }

    /// Ask whether to save, discard or cancel before closing.
    fn prompt_save_then_close(
        &mut self,
        document: Entity<DocumentPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use gpui::component::{
            WindowExt as _,
            button::Button,
            dialog::{DialogAction, DialogClose, DialogFooter},
        };

        let name = crate::workspace::fs_util::file_name_of(document.read(cx).path());
        // The dialog's builder and its callbacks are all `Fn`, so every captured
        // value is cloned out of the shared state rather than moved.
        let for_discard = std::rc::Rc::new(document.clone());
        let for_save = std::rc::Rc::new(document);
        let shell = cx.weak_entity();
        let shell_for_discard = shell.clone();

        window.open_dialog(cx, move |dialog, _, _| {
            let discard_document = for_discard.clone();
            let discard_shell = shell_for_discard.clone();
            let save_document = for_save.clone();
            let save_shell = shell.clone();
            dialog
                .title(SharedString::from("Unsaved changes"))
                .child(SharedString::from(format!(
                    "{name} has unsaved changes. Save before closing?"
                )))
                .footer(
                    DialogFooter::new()
                        .child(DialogClose::new().trigger(|button| button.label("Cancel")))
                        .child(Button::new("discard").label("Discard").on_click(
                            move |_, window, cx| {
                                let document = discard_document.clone();
                                let _ = discard_shell.update(cx, |shell, cx| {
                                    shell.remove_document(&document, window, cx);
                                });
                                window.close_dialog(cx);
                            },
                        ))
                        .child(
                            DialogAction::new().child(Button::new("save").label("Save").primary()),
                        ),
                )
                .on_ok(move |_, window, cx| {
                    let document = save_document.clone();
                    let saved =
                        document.update(cx, |document, cx| document.controller().save(window, cx));
                    match saved {
                        Ok(()) => {
                            let _ = save_shell.update(cx, |shell, cx| {
                                shell.remove_document(&document, window, cx);
                            });
                            true
                        }
                        Err(error) => {
                            eprintln!("{error}");
                            false
                        }
                    }
                })
        });
    }

    /// Drop a document's tab and its record.
    fn remove_document(
        &mut self,
        document: &Entity<DocumentPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panel = document.clone();
        self.dock.update(cx, |dock, cx| {
            dock.remove_panel(panel, window, cx);
        });

        self.documents.retain(|open| open != document);
        if self
            .active_document
            .as_ref()
            .is_some_and(|active| active == document)
        {
            self.active_document = self.documents.last().cloned();
        }
        cx.notify();
    }

    /// Close every document, discarding changes.
    fn close_all_documents(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for document in std::mem::take(&mut self.documents) {
            self.dock.update(cx, |dock, cx| {
                dock.remove_panel(document, window, cx);
            });
        }
        self.active_document = None;
        cx.notify();
    }

    /// Keep `active_document` in step with the dock's selected tab.
    fn sync_active_document(&mut self, cx: &mut Context<Self>) {
        let tree = self.dock.read(cx).layout(DockPlacement::Center).cloned();
        let Some(tree) = tree else {
            return;
        };
        let Some(active_id) = active_panel_in_tree(&tree) else {
            return;
        };
        let found = self
            .documents
            .iter()
            .find(|document| PanelId::from(document.entity_id()) == active_id)
            .cloned();
        if let Some(document) = found {
            self.active_document = Some(document);
        }
    }

    /// Show a side pane by provider id.
    pub fn show_pane(
        &mut self,
        slot: PaneSlot,
        provider_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(provider) = panes::pane_by_id(cx, provider_id) else {
            return;
        };
        self.active_panes.insert(slot, provider_id.to_owned());

        // Select the pane's tab if it is mounted; otherwise mount it.
        let existing = self
            .dock
            .read(cx)
            .layout(slot.placement())
            .map(panel_ids_in_tree)
            .unwrap_or_default()
            .into_iter()
            .find(|id| {
                self.dock
                    .read(cx)
                    .panel(*id)
                    .is_some_and(|panel| panel.panel_name(cx) == provider.id())
            });

        match existing {
            Some(id) => {
                self.dock.update(cx, |dock, cx| {
                    dock.select_panel(id, window, cx);
                    if !dock.is_dock_open(slot.placement()) {
                        dock.toggle_dock(slot.placement(), window, cx);
                    }
                });
            }
            None => {
                let panel = provider.build(
                    panes::PaneContext {
                        workspace: self.workspace.clone(),
                        shell: cx.weak_entity(),
                        saved: None,
                    },
                    window,
                    cx,
                );
                self.dock.update(cx, |dock, cx| {
                    dock.add_panel_view(panel, slot.placement(), None, window, cx);
                });
            }
        }
        cx.notify();
    }

    /// Toggle a side dock open or closed.
    pub fn toggle_dock(&mut self, slot: PaneSlot, window: &mut Window, cx: &mut Context<Self>) {
        self.dock.update(cx, |dock, cx| {
            dock.toggle_dock(slot.placement(), window, cx);
        });
        cx.notify();
    }

    /// Whether a side dock is open.
    pub fn is_dock_open(&self, slot: PaneSlot, cx: &App) -> bool {
        self.dock.read(cx).is_dock_open(slot.placement())
    }

    /// Navigate the browser pane to `url`.
    ///
    /// The video viewer's "open in browser" button goes through this rather than
    /// reaching into the browser pane, so a viewer never needs to know which
    /// pane can play a file. The request is recorded and delivered to the
    /// browser pane, which reads it when it next renders.
    pub fn navigate_browser(&mut self, url: String, cx: &mut Context<Self>) {
        self.browser_request = Some(url);
        cx.notify();
    }

    /// Take the pending browser navigation, if any.
    pub fn take_browser_request(&mut self) -> Option<String> {
        self.browser_request.take()
    }

    /// Ask for a folder and open it.
    pub fn pick_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace
            .update(cx, |workspace, cx| workspace.pick_root(window, cx));
    }

    /// Re-read the tree.
    pub fn refresh_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace
            .update(cx, |workspace, cx| workspace.refresh(window, cx));
    }

    /// Write the session after a short delay.
    ///
    /// A layout change arrives once per frame while a divider is dragged, so
    /// writing on every event would rewrite the file dozens of times per
    /// gesture. The debounce coalesces them; the newest write wins because each
    /// task reads the layout when it fires rather than when it was spawned.
    fn schedule_session_save(&mut self, window: &Window, cx: &mut Context<Self>) {
        // A save already pending covers this change too.
        if self.save_task.is_some() {
            return;
        }
        let this = cx.weak_entity();
        let task = cx.spawn_in(window, async move |_, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(500))
                .await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.save_task = None;
                let session = this.dump_session(cx);
                crate::state::save(&session);
            });
        });
        self.save_task = Some(task);
    }

    /// The session to write on quit.
    pub fn dump_session(&self, cx: &App) -> SessionState {
        SessionState {
            version: crate::state::SESSION_VERSION,
            workspace_root: self
                .workspace
                .read(cx)
                .root()
                .map(|path| path.to_path_buf()),
            dock: Some(self.dock.read(cx).dump(cx)),
            active_left: self.active_panes.get(&PaneSlot::Left).cloned(),
            active_right: self.active_panes.get(&PaneSlot::Right).cloned(),
            window: None,
        }
    }
}

impl Render for AppShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = window;
        let left_active = self.active_panes.get(&PaneSlot::Left).cloned();
        let right_active = self.active_panes.get(&PaneSlot::Right).cloned();
        let dock = self.dock.clone();
        let left_open = self.is_dock_open(PaneSlot::Left, cx);
        let right_open = self.is_dock_open(PaneSlot::Right, cx);

        // Everything read from the app is resolved into owned values first.
        // The listeners below each need `cx` mutably, and one immutable borrow
        // of the theme cannot overlap another.
        struct Chrome {
            background: gpui::Background,
            foreground: gpui::Hsla,
            font_family: SharedString,
            font_size: gpui::Pixels,
            border: gpui::Hsla,
            sidebar: gpui::Hsla,
        }
        let chrome = {
            let theme = cx.theme();
            Chrome {
                background: theme.tokens.background.background,
                foreground: theme.foreground,
                font_family: theme.font_family.clone(),
                font_size: theme.font_size,
                border: theme.border,
                sidebar: theme.tokens.sidebar.color,
            }
        };
        let left_providers = panes::panes_for(cx, PaneSlot::Left);
        let right_providers = panes::panes_for(cx, PaneSlot::Right);

        let title = title_bar::render(
            cx.listener(|this, _, window, cx| this.pick_folder(window, cx)),
            cx.listener(|this, _, window, cx| this.refresh_tree(window, cx)),
            cx,
        );
        let status = status_bar::render(self, cx);

        let left_strip = ActivityBar::new(PaneSlot::Left, left_active, left_providers)
            .on_select(cx.listener(|this, id: &SharedString, window, cx| {
                this.show_pane(PaneSlot::Left, id, window, cx);
            }))
            .on_toggle(cx.listener(|this, slot: &PaneSlot, window, cx| {
                this.toggle_dock(*slot, window, cx);
            }))
            .set_open(left_open)
            .render(chrome.border, chrome.sidebar);
        let right_strip = ActivityBar::new(PaneSlot::Right, right_active, right_providers)
            .on_select(cx.listener(|this, id: &SharedString, window, cx| {
                this.show_pane(PaneSlot::Right, id, window, cx);
            }))
            .on_toggle(cx.listener(|this, slot: &PaneSlot, window, cx| {
                this.toggle_dock(*slot, window, cx);
            }))
            .set_open(right_open)
            .render(chrome.border, chrome.sidebar);

        rsx! {
            <div
                class="flex flex-col size-full"
                bg={chrome.background}
                text_color={chrome.foreground}
                font_family={chrome.font_family}
                text_size={chrome.font_size}
            >
                {title}
                <div class="flex flex-row flex-1 min-h-0 items-stretch">
                    {left_strip}
                    <div class="flex-1 min-w-0">{dock.clone()}</div>
                    {right_strip}
                </div>
                {status}
            </div>
        }
    }
}

/// Every panel id in a tree, in tab order.
fn panel_ids_in_tree(tree: &gpui::base::dock::PaneTree) -> Vec<PanelId> {
    let mut ids = Vec::new();
    collect_panel_ids(tree.root(), &mut ids);
    ids
}

fn collect_panel_ids(node: &gpui::base::dock::PaneNode, ids: &mut Vec<PanelId>) {
    match node.kind() {
        gpui::base::dock::PaneRef::Split { children, .. } => {
            for child in children {
                collect_panel_ids(child, ids);
            }
        }
        gpui::base::dock::PaneRef::Tabs { panels, .. } => ids.extend(panels.iter().copied()),
    }
}

/// The panel id of the selected tab in a tree, if any.
fn active_panel_in_tree(tree: &gpui::base::dock::PaneTree) -> Option<PanelId> {
    fn walk(node: &gpui::base::dock::PaneNode) -> Option<PanelId> {
        match node.kind() {
            gpui::base::dock::PaneRef::Split { children, .. } => children.iter().find_map(walk),
            gpui::base::dock::PaneRef::Tabs { panels, active_ix } => panels.get(active_ix).copied(),
        }
    }
    walk(tree.root())
}

/// Whether any document has unsaved changes, for the quit guard.
pub fn any_dirty(shell: &AppShell, cx: &App) -> bool {
    shell
        .documents()
        .iter()
        .any(|document| document.read(cx).is_dirty(cx))
}
