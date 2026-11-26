//! The application shell.
//!
//! The shell knows three seams and nothing else: [`PaneRegistry`] for the side
//! slots, [`DocumentRegistry`] for the centre, and [`Workspace`] for the folder.
//! It has no idea what a file, an editor or a webview is — which is what makes
//! the panes and the viewers replaceable without touching this file.
//!
//! What it does own is the *arrangement*: two [`SideView`]s and a
//! [`DocumentsView`] placed in one resizable group, plus everything that follows
//! from a document opening, closing or being replaced — the tree reveal, the
//! `ActiveDocumentChanged` event, the session write. The views draw themselves;
//! they report the change and the shell decides what it means.
//!
//! The activity strips sit outside the resizable group on purpose: a collapsed
//! dock must not take the button that reopens it with it.
//!
//! # Why a few accessors are `pub` but unused by the binary
//!
//! [`AppShell::active_panes`], [`AppShell::toggle_dock`],
//! [`AppShell::is_dock_open`], [`AppShell::pick_folder`] and
//! [`AppShell::refresh_tree`] are the shell's own seam: a test drives the dock
//! through them, and a later feature (a keyboard shortcut, a menu) would too.
//! The dead-code lint cannot see a test-only consumer, so it is allowed on those
//! items rather than hiding the shell's state behind an incomplete API.

use std::{collections::HashMap, path::PathBuf};

use gpui::component::{
    ActiveTheme as _, Sizable as _,
    resizable::{ResizablePanelEvent, ResizableState, h_resizable, resizable_panel},
};
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, Window, div, px,
};
use gpui_rsx::rsx;

use crate::{
    app::{
        dialog_window::{self, DialogAction, DialogOptions},
        documents_view::DocumentsView,
        side_view::SideView,
        status_bar, title_bar,
    },
    documents::DocumentPanel,
    panes::{self, PaneSlot},
    state::{SessionState, SideState},
    workspace::{Workspace, WorkspaceEvent, fs_util},
};

/// The width the left dock gets when nothing else says otherwise.
const LEFT_WIDTH: gpui::Pixels = px(260.);
/// The width the right dock gets for the same reason.
const RIGHT_WIDTH: gpui::Pixels = px(420.);

/// How narrow each side dock may be dragged, and how wide.
const LEFT_RANGE: std::ops::Range<gpui::Pixels> = px(160.)..px(520.);
const RIGHT_RANGE: std::ops::Range<gpui::Pixels> = px(260.)..px(800.);

/// The application shell.
pub struct AppShell {
    focus_handle: FocusHandle,
    /// The folder, its tree and its watcher.
    workspace: Entity<Workspace>,
    /// The centre: the document tabs and the document on show.
    documents: DocumentsView,
    /// The two side docks, by slot.
    sides: HashMap<PaneSlot, SideView>,
    /// The widths of the three columns, so a drag and a restart agree.
    columns: Entity<ResizableState>,
    /// The file last revealed in the tree.
    ///
    /// The reveal must not scroll and rebuild the tree on every frame of a
    /// divider drag; this is what makes it happen only when the displayed *file*
    /// actually changed.
    revealed: Option<PathBuf>,
    /// A URL the browser pane should load next, put here by a viewer that cannot
    /// play a file itself.
    browser_request: Option<String>,
    /// The pending debounced session write, held so a newer change can reuse it.
    save_task: Option<gpui::Task<()>>,
    /// Dropped with the shell, so the subscriptions do not outlive it.
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<()> for AppShell {}

/// The shell's own event: the document on show changed.
///
/// A pane that follows the active document — the outline is the one that does —
/// subscribes to this rather than re-reading the shell on a timer. The event
/// carries nothing: the subscriber reads [`AppShell::active_document`], so there
/// is no second copy of the shell's state to fall out of step.
#[derive(Clone, Copy, Debug)]
pub struct ActiveDocumentChanged;

impl EventEmitter<ActiveDocumentChanged> for AppShell {}

/// What the save prompt was answered with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SaveAnswer {
    Cancel,
    Discard,
    Save,
}

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
        let columns = cx.new(|_| ResizableState::default());

        let shell = cx.new(|cx| Self {
            focus_handle: cx.focus_handle(),
            workspace: workspace.clone(),
            documents: DocumentsView::default(),
            sides: HashMap::new(),
            columns: columns.clone(),
            revealed: None,
            browser_request: None,
            save_task: None,
            _subscriptions: Vec::new(),
        });

        shell.update(cx, |shell, cx| {
            shell.subscribe_all(window, cx);
            shell.restore_session(session, window, cx);
        });

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
    /// to prompt in by the time this runs (risk R11).
    pub fn install_quit_hook(&mut self, cx: &mut Context<Self>) {
        // The hook is handed the shell itself. Reading it through a weak handle
        // would re-enter the very `update` that is running the hook, which panics
        // with "cannot read AppShell while it is already being updated".
        let subscription = cx.on_app_quit(|shell: &mut Self, cx| {
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
            async {}
        });
        self._subscriptions.push(subscription);
    }

    /// The workspace folder.
    pub fn workspace(&self) -> &Entity<Workspace> {
        &self.workspace
    }

    /// The documents currently open, in tab order.
    pub fn documents(&self) -> &[Entity<DocumentPanel>] {
        self.documents.documents()
    }

    /// The document the centre is showing.
    pub fn active_document(&self, cx: &App) -> Option<Entity<DocumentPanel>> {
        let _ = cx;
        self.documents.active()
    }

    /// Which provider each side dock is showing.
    #[allow(dead_code)]
    pub fn active_panes(&self) -> HashMap<PaneSlot, String> {
        self.sides
            .iter()
            .filter_map(|(slot, side)| side.active().map(|id| (*slot, id.to_owned())))
            .collect()
    }

    fn subscribe_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace_subscription = cx.subscribe_in(
            &self.workspace,
            window,
            |this: &mut Self, _, event: &WorkspaceEvent, window, cx| {
                this.on_workspace_event(event, window, cx);
            },
        );

        // A dragged divider moves the columns; nothing else tells a subscriber
        // to persist the new widths.
        let columns_subscription = cx.subscribe_in(
            &self.columns,
            window,
            |this: &mut Self, _, event: &ResizablePanelEvent, window, cx| {
                if matches!(event, ResizablePanelEvent::Resized) {
                    this.schedule_session_save(window, cx);
                }
            },
        );

        self._subscriptions = vec![workspace_subscription, columns_subscription];
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
                self.default_layout(window, cx);
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
                .documents()
                .iter()
                .filter(|document| fs_util::same_path(document.read(cx).path(), path))
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

    /// The dock for `slot`, created on first use.
    fn side(&mut self, slot: PaneSlot, cx: &mut Context<Self>) -> &mut SideView {
        self.sides
            .entry(slot)
            .or_insert_with(|| SideView::new(slot, self.workspace.clone(), cx.weak_entity()))
    }

    /// Install the layout a fresh session starts with: the explorer on the left,
    /// the browser on the right, both open, and no document.
    fn default_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sides.clear();
        for slot in PaneSlot::ALL {
            let first = panes::panes_for(cx, slot)
                .first()
                .map(|provider| provider.id().to_owned());
            let side = self.side(slot, cx);
            match first {
                Some(id) => {
                    side.show(&id, window, cx);
                }
                None => side.apply(&SideState::default(), cx),
            }
            // A closed dock is reopened by its strip, and the strip reads the
            // active pane, so the pane has to exist before the first frame.
            side.ensure_active(window, cx);
        }
    }

    /// Restore the saved session, falling back to the default layout.
    fn restore_session(
        &mut self,
        session: SessionState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The workspace first: the panes read it as they are built.
        if let Some(root) = session.workspace_root.clone()
            && root.exists()
        {
            self.workspace
                .update(cx, |workspace, cx| workspace.set_root(root, window, cx));
        }

        for slot in PaneSlot::ALL {
            let saved = match slot {
                PaneSlot::Left => &session.left,
                PaneSlot::Right => &session.right,
            };
            let side = self.side(slot, cx);
            side.apply(saved, cx);
            side.ensure_active(window, cx);
        }

        for saved in &session.documents {
            match DocumentPanel::open(
                saved.path.clone(),
                cx.weak_entity(),
                self.workspace.clone(),
                window,
                cx,
            ) {
                Some(document) => self.documents.push(document),
                // The file or its viewer is gone. The tab is kept rather than
                // dropped, because the session named it (risk R12).
                None => {
                    eprintln!("could not reopen {}", saved.path.display());
                    self.documents
                        .push(DocumentPanel::missing(saved.path.clone(), cx));
                }
            }
        }
        self.documents.restore_active(session.active_document);

        if let Some(document) = self.active_document(cx) {
            let controller = document.read(cx).controller().clone();
            controller.focus(window, cx);
            let path = document.read(cx).path().to_path_buf();
            self.reveal(&path, window, cx);
        }
    }

    /// Open a file in the centre.
    pub fn open_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let canonical = std::fs::canonicalize(&path).unwrap_or(path);

        // A tab for this file already exists: select it rather than duplicate.
        let existing = self
            .documents()
            .iter()
            .position(|document| fs_util::same_path(document.read(cx).path(), &canonical));
        if let Some(ix) = existing {
            self.select_document(ix, window, cx);
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

        self.documents.push(panel);
        if let Some(document) = self.active_document(cx) {
            let controller = document.read(cx).controller().clone();
            controller.focus(window, cx);
        }
        self.reveal(&canonical, window, cx);
        cx.emit(ActiveDocumentChanged);
        self.schedule_session_save(window, cx);
        cx.notify();
    }

    /// Show the document at `ix` in the centre.
    fn select_document(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.documents.select(ix) {
            return;
        }
        // Selecting a tab is also focusing it: the editor should take the
        // keyboard, or the user's next keystroke goes to the last thing that had
        // it.
        if let Some(document) = self.active_document(cx) {
            let controller = document.read(cx).controller().clone();
            controller.focus(window, cx);
            let path = document.read(cx).path().to_path_buf();
            self.reveal(&path, window, cx);
        }
        cx.emit(ActiveDocumentChanged);
        self.schedule_session_save(window, cx);
        cx.notify();
    }

    /// Show `path` in the explorer's tree, if the tree moved.
    ///
    /// Silent about a dock the user has collapsed or switched to another pane:
    /// the tree is kept in step either way, but revealing is not an excuse to
    /// reopen a dock someone put away.
    fn reveal(&mut self, path: &std::path::Path, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .revealed
            .as_deref()
            .is_some_and(|last| fs_util::same_path(last, path))
        {
            return;
        }
        self.revealed = Some(path.to_path_buf());
        self.workspace
            .update(cx, |workspace, cx| workspace.reveal(path, window, cx));
    }

    /// Close one document, prompting if it has unsaved changes.
    ///
    /// This is the path a keyboard shortcut, a menu and the tab's own close
    /// button all take.
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

    /// Ask whether to save, discard or cancel before closing.
    ///
    /// The prompt is its own window: an in-window dialog is covered by the
    /// browser pane's native child window (risk R1). Opening a window inside
    /// this update would paint the new window's first frame mid-update, so the
    /// window is opened once this one has finished drawing.
    fn prompt_save_then_close(
        &mut self,
        document: Entity<DocumentPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.defer_in(window, move |this, window, cx| {
            this.show_save_prompt(document, window, cx)
        });
    }

    fn show_save_prompt(
        &mut self,
        document: Entity<DocumentPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = fs_util::file_name_of(document.read(cx).path());
        let options = DialogOptions {
            title: SharedString::from("Unsaved changes"),
            message: Some(SharedString::from(format!(
                "{name} has unsaved changes. Save before closing?"
            ))),
            ..Default::default()
        };
        let actions = vec![
            DialogAction {
                id: "cancel",
                label: SharedString::from("Cancel"),
                primary: false,
                value: SaveAnswer::Cancel,
            },
            DialogAction {
                id: "discard",
                label: SharedString::from("Discard"),
                primary: false,
                value: SaveAnswer::Discard,
            },
            DialogAction {
                id: "save",
                label: SharedString::from("Save"),
                primary: true,
                value: SaveAnswer::Save,
            },
        ];

        let answered = dialog_window::open(window, cx, options, actions);
        // The prompt acts on the owner window — the tree reveal and the editor
        // focus both need it — so the answer comes back through its handle.
        let owner = window.window_handle();
        let shell = cx.weak_entity();
        cx.spawn_in(window, async move |_, cx| {
            let Some(answer) = answered.await else {
                return;
            };
            let _ = owner.update(cx, |_, window, cx| {
                let _ = shell.update(cx, |shell, cx| {
                    shell.apply_save_answer(answer, &document, window, cx)
                });
            });
        })
        .detach();
    }

    /// Do what the save prompt asked for.
    ///
    /// A failed save cannot keep the prompt open — it is already gone — so it
    /// reports in a second window and leaves the document dirty and open.
    fn apply_save_answer(
        &mut self,
        answer: SaveAnswer,
        document: &Entity<DocumentPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match answer {
            SaveAnswer::Cancel => {}
            SaveAnswer::Discard => self.remove_document(document, window, cx),
            SaveAnswer::Save => {
                let saved =
                    document.update(cx, |document, cx| document.controller().save(window, cx));
                match saved {
                    Ok(()) => self.remove_document(document, window, cx),
                    Err(error) => {
                        dialog_window::alert(window, cx, "Could not save", error);
                    }
                }
            }
        }
    }

    /// Drop a document's tab, and follow whatever the centre now shows.
    fn remove_document(
        &mut self,
        document: &Entity<DocumentPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(next) = self.documents.remove(document) else {
            // Nothing left to show.
            self.revealed = None;
            cx.emit(ActiveDocumentChanged);
            self.schedule_session_save(window, cx);
            cx.notify();
            return;
        };

        // The tab that took its place is now what the centre shows, so the tree
        // and the status bar have to hear about it.
        let path = next.read(cx).path().to_path_buf();
        self.reveal(&path, window, cx);
        cx.emit(ActiveDocumentChanged);
        self.schedule_session_save(window, cx);
        cx.notify();
    }

    /// Close every document, discarding changes.
    fn close_all_documents(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.documents.clear();
        self.revealed = None;
        self.schedule_session_save(window, cx);
        cx.emit(ActiveDocumentChanged);
        cx.notify();
    }

    /// Show a side pane by provider id, or hide the dock if it is already
    /// showing it.
    pub fn show_pane(
        &mut self,
        slot: PaneSlot,
        provider_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.side(slot, cx).show(provider_id, window, cx) {
            return;
        }
        self.schedule_session_save(window, cx);
        cx.notify();
    }

    /// Toggle a side dock open or closed.
    #[allow(dead_code)]
    pub fn toggle_dock(&mut self, slot: PaneSlot, window: &mut Window, cx: &mut Context<Self>) {
        self.side(slot, cx).toggle(window, cx);
        self.schedule_session_save(window, cx);
        cx.notify();
    }

    /// Whether a side dock is open.
    #[allow(dead_code)]
    pub fn is_dock_open(&self, slot: PaneSlot, cx: &App) -> bool {
        let _ = cx;
        self.sides.get(&slot).is_some_and(|side| side.is_open())
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
    #[allow(dead_code)]
    pub fn pick_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace
            .update(cx, |workspace, cx| workspace.pick_root(window, cx));
    }

    /// Re-read the tree.
    #[allow(dead_code)]
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
            // Columns: left dock, centre, right dock.
            left: self.side_state(PaneSlot::Left, 0, cx),
            right: self.side_state(PaneSlot::Right, 2, cx),
            documents: self
                .documents()
                .iter()
                .map(|document| crate::state::SavedDocument {
                    path: document.read(cx).path().to_path_buf(),
                })
                .collect(),
            active_document: self.documents.saved_active(),
            window: None,
        }
    }

    /// One dock's state, with the width the columns measured.
    fn side_state(&self, slot: PaneSlot, ix: usize, cx: &App) -> SideState {
        let width = self
            .columns
            .read(cx)
            .sizes()
            .get(ix)
            .map(|size| size.as_f32())
            .filter(|width| *width > 1.);
        match self.sides.get(&slot) {
            Some(side) => side.state(width),
            None => SideState::default(),
        }
    }

    /// The width to lay a dock out at on the first frame.
    fn column_size(
        &self,
        slot: PaneSlot,
        ix: usize,
        fallback: gpui::Pixels,
        cx: &App,
    ) -> gpui::Pixels {
        self.columns
            .read(cx)
            .sizes()
            .get(ix)
            .copied()
            .filter(|size| *size > px(1.))
            .or_else(|| self.sides.get(&slot).and_then(|side| side.saved_width()))
            .unwrap_or(fallback)
    }
}

/// A framed area of the window: an optional header strip above a body.
///
/// The side docks and the centre are the same shape, which is why they share
/// this rather than each drawing their own frame.
pub(crate) fn frame(
    id: &'static str,
    header: Option<AnyElement>,
    body: AnyElement,
    cx: &App,
) -> AnyElement {
    let background = cx.theme().tokens.background;

    rsx! {
        <div
            id={SharedString::from(id)}
            class="flex flex-col size-full min-w-0 min-h-0"
            bg={background}
        >
            {...header.into_iter()}
            <div class="flex-1 min-h-0 overflow-hidden">{body}</div>
        </div>
    }
    .into_any_element()
}

/// What a pane shows when it has nothing to show.
pub(crate) fn empty_state(title: &str, hint: &str, cx: &App) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let foreground = cx.theme().foreground;
    let title = SharedString::from(title.to_owned());
    let hint = SharedString::from(hint.to_owned());

    rsx! {
        <div class="flex flex-col size-full items-center justify-center gap-2 p-6">
            {gpui::component::Icon::new(gpui::component::IconName::FolderOpen)
                .large()
                .text_color(muted)}
            <div class="text-sm" text_color={foreground}>{title}</div>
            <div class="text-xs" text_color={muted}>{hint}</div>
        </div>
    }
    .into_any_element()
}

impl Render for AppShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = window;
        // Widths first: reading them borrows the shell immutably, and `side`
        // below hands out a mutable borrow of the dock.
        let left_width = self.column_size(PaneSlot::Left, 0, LEFT_WIDTH, cx);
        let right_width = self.column_size(PaneSlot::Right, 2, RIGHT_WIDTH, cx);

        let left = self.side(PaneSlot::Left, cx);
        let left_strip = left.render_strip(cx);
        let left_open = left.is_open();
        let left_content = left.render_content(cx);

        let right = self.side(PaneSlot::Right, cx);
        let right_strip = right.render_strip(cx);
        let right_open = right.is_open();
        let right_content = right.render_content(cx);

        let documents = self.documents.render(
            cx.listener(|this: &mut AppShell, ix: &usize, window, cx| {
                this.select_document(*ix, window, cx)
            }),
            cx.listener(|this: &mut AppShell, ix: &usize, window, cx| {
                if let Some(document) = this.documents().get(*ix).cloned() {
                    this.close_document(document, window, cx);
                }
            }),
            cx,
        );

        // Everything read from the app is resolved into owned values first.
        // The listeners above each need `cx` mutably, and one immutable borrow
        // of the theme cannot overlap another.
        struct Chrome {
            background: gpui::Background,
            foreground: gpui::Hsla,
            font_family: SharedString,
            font_size: gpui::Pixels,
        }
        let chrome = {
            let theme = cx.theme();
            Chrome {
                background: theme.tokens.background.background,
                foreground: theme.foreground,
                font_family: theme.font_family.clone(),
                font_size: theme.font_size,
            }
        };

        let title = title_bar::render();
        let status = status_bar::render(self, cx);
        let columns = self.columns.clone();

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
                    <div class="flex-1 min-w-0">
                        <h_resizable("workspace-columns") with_state={&columns}>
                            <resizable_panel
                                base={resizable_panel().visible(left_open)}
                                size={left_width}
                                size_range={LEFT_RANGE}
                                flex_none
                            >
                                {left_content}
                            </resizable_panel>
                            <resizable_panel>
                                {documents}
                            </resizable_panel>
                            <resizable_panel
                                base={resizable_panel().visible(right_open)}
                                size={right_width}
                                size_range={RIGHT_RANGE}
                                flex_none
                            >
                                {right_content}
                            </resizable_panel>
                        </h_resizable>
                    </div>
                    {right_strip}
                </div>
                {status}
            </div>
        }
    }
}

/// Whether any document has unsaved changes, for the quit guard.
pub fn any_dirty(shell: &AppShell, cx: &App) -> bool {
    shell
        .documents()
        .iter()
        .any(|document| document.read(cx).is_dirty(cx))
}
