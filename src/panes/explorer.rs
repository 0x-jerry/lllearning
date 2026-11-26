//! The explorer: the workspace folder's tree.
//!
//! A toolbar with the folder name and the actions that change it, then the
//! lazily-loaded tree. Expanding a directory asks the workspace to read it;
//! clicking a file asks the shell to open it.

use gpui::base::TreeEvent;
use gpui::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
};
use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Subscription, Window, div, px,
};
use gpui_rsx::rsx;

use crate::{
    app::shell::AppShell,
    icons,
    panes::{PaneContent, PaneContext, PaneProvider, PaneSlot},
    workspace::{Workspace, fs_util},
};

/// The explorer's provider.
pub struct ExplorerPane;

impl PaneProvider for ExplorerPane {
    fn id(&self) -> &'static str {
        "explorer"
    }

    fn title(&self) -> SharedString {
        SharedString::from("Explorer")
    }

    fn icon(&self) -> lucide_icons::Icon {
        lucide_icons::Icon::Files
    }

    fn slot(&self) -> PaneSlot {
        PaneSlot::Left
    }

    fn order(&self) -> i32 {
        0
    }

    fn build(&self, ctx: PaneContext, window: &mut Window, cx: &mut App) -> PaneContent {
        let view = cx.new(|cx| ExplorerView::new(ctx.workspace, ctx.shell, window, cx));
        PaneContent::new(view)
    }
}

/// The explorer's state.
pub struct ExplorerView {
    focus_handle: FocusHandle,
    workspace: Entity<Workspace>,
    shell: gpui::WeakEntity<AppShell>,
    /// Dropped with the view, so the tree subscription does not outlive it.
    _tree_events: Subscription,
}

impl ExplorerView {
    fn new(
        workspace: Entity<Workspace>,
        shell: gpui::WeakEntity<AppShell>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        let tree_state = workspace.read(cx).tree_state().clone();
        // Both halves of a toggle arrive here: the tree flips the row and says
        // which way, and the workspace keeps its cache in step. Ignoring
        // `Collapsed` would leave the cache believing a folded directory is
        // still open, and the next click would look like it did nothing.
        let tree_events = cx.subscribe_in(
            &tree_state,
            window,
            |this: &mut Self, _, event: &TreeEvent, window, cx| match event {
                TreeEvent::Expanded(id) => this.on_toggled(id, true, window, cx),
                TreeEvent::Collapsed(id) => this.on_toggled(id, false, window, cx),
            },
        );

        Self {
            focus_handle,
            workspace,
            shell,
            _tree_events: tree_events,
        }
    }

    /// Read a directory's children when the tree opens it, and record what the
    /// click did either way.
    fn on_toggled(
        &mut self,
        id: &SharedString,
        expanded: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if crate::workspace::tree::DirTree::is_placeholder_id(id) {
            return;
        }
        let path = crate::workspace::tree::DirTree::path_for_id(id);
        self.workspace.update(cx, |workspace, cx| {
            workspace.toggle_dir(path, expanded, window, cx)
        });
        cx.notify();
    }

    fn pick_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace
            .update(cx, |workspace, cx| workspace.pick_root(window, cx));
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace
            .update(cx, |workspace, cx| workspace.refresh(window, cx));
    }

    /// The folder name, or a placeholder when none is open.
    fn root_label(&self, cx: &App) -> SharedString {
        match self.workspace.read(cx).root() {
            Some(path) => SharedString::from(fs_util::file_name_of(path)),
            None => SharedString::from("No folder open"),
        }
    }
}

impl Focusable for ExplorerView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ExplorerView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let label = self.root_label(cx);
        let tree_state = self.workspace.read(cx).tree_state().clone();
        let has_root = self.workspace.read(cx).root().is_some();
        let muted = cx.theme().muted_foreground;
        let shell = self.shell.clone();

        rsx! {
            <div class="flex flex-col size-full">
                <div
                    class="flex flex-row items-center justify-between gap-1 px-2 py-1"
                    border_b_1
                    border_color={cx.theme().border}
                >
                    <div class="flex flex-row items-center gap-1 min-w-0">
                        {icons::icon(lucide_icons::Icon::FolderOpen, 14.)}
                        <div class="text-xs truncate">{label}</div>
                    </div>
                    <div class="flex flex-row items-center gap-1">
                        <Button("explorer-pick")
                            ghost
                            xsmall
                            tooltip={"Change folder…"}
                            onClick={cx.listener(|this, _, window, cx| {
                                this.pick_folder(window, cx)
                            })}
                        >
                            {icons::icon(lucide_icons::Icon::FolderOpen, 14.)}
                        </Button>
                        <Button("explorer-refresh")
                            ghost
                            xsmall
                            tooltip={"Refresh"}
                            onClick={cx.listener(|this, _, window, cx| {
                                this.refresh(window, cx)
                            })}
                        >
                            {icons::icon(lucide_icons::Icon::RefreshCw, 14.)}
                        </Button>
                    </div>
                </div>
                <div class="flex-1 min-h-0 overflow-hidden">
                    {if has_root {
                        rsx! {
                            <div class="size-full">
                                {gpui::component::tree::tree(&tree_state, move |_ix, entry, _selected, _window, cx| {
                                    row(entry, &shell, cx)
                                })}
                            </div>
                        }
                    } else {
                        rsx! {
                            <div class="p-2 text-xs" text_color={muted}>
                                {"Choose a folder to browse its files."}
                            </div>
                        }
                    }}
                </div>
            </div>
        }
    }
}

/// One tree row: an indent, a file-type icon, and the name.
///
/// The click is on the label, not the row, and it stops propagation: the base
/// tree toggles a directory on any click, so a file's click has to be claimed
/// before it reaches that handler (risk R6).
fn row(
    entry: &gpui::base::TreeEntry,
    shell: &gpui::WeakEntity<AppShell>,
    cx: &mut App,
) -> gpui::component::list::ListItem {
    use gpui::component::list::ListItem;

    let item = entry.item();
    if crate::workspace::tree::DirTree::is_placeholder_id(&item.id) {
        return ListItem::new(item.id.clone()).px(px(0.));
    }

    let path = crate::workspace::tree::DirTree::path_for_id(&item.id);
    let is_dir = entry.is_folder();
    let expanded = entry.is_expanded();
    let depth = entry.depth();
    let foreground = cx.theme().foreground;

    // A directory's folder glyph doubles as its expand/collapse indicator: the
    // open and closed folders say the state, so a separate chevron would only
    // repeat it. A file gets its language's glyph, from the nerd font.
    let icon = if is_dir {
        icons::folder_icon(expanded, 12.)
    } else {
        icons::file_icon(&path, 12., cx)
    };

    let label = item.label.clone();
    let shell_for_click = shell.clone();
    let path_for_click = path.clone();

    let content = rsx! {
        <div class="flex flex-row items-center gap-1 w-full" h={px(22.)}>
            <div pl={px(4. + 10. * depth as f32)} />
            {icon}
            <div class="text-xs truncate" pl={px(2.)} text_color={foreground}>{label}</div>
        </div>
    };

    // The click goes on the whole row, not on the label: a click anywhere in a
    // file's row should open it, and the row is what the pointer actually
    // covers. A directory's click falls through to the tree, which toggles it
    // (risk R6 — the row handler runs first, so a file claims the click before
    // the tree's any-click toggle can fire).
    let list_item = ListItem::new(item.id.clone()).child(content);
    if is_dir {
        list_item
    } else {
        list_item.on_click(move |_, window, cx| {
            cx.stop_propagation();
            let path = path_for_click.clone();
            let _ = shell_for_click.update(cx, |shell, cx| {
                shell.open_path(path, window, cx);
            });
        })
    }
}
