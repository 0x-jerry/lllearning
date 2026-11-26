//! The outline pane: whatever the active document's viewer says its structure
//! is.
//!
//! This pane knows no syntax. A viewer answers
//! [`DocumentController::outline`](crate::documents::DocumentController::outline)
//! and the pane draws the answer, which is why markdown's headings and a future
//! viewer's symbols are both one method on the viewer rather than a branch here.
//! A viewer that cannot outline itself answers `None`, and the pane says so
//! instead of inventing a list from the file's text.
//!
//! The pane is a pure function of the shell's active document plus its own
//! observation of it: it never reads the file from disk, because the viewer's
//! buffer is the truth and the disk copy may be behind it.

use gpui::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
};
use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, div, px,
};
use gpui_rsx::rsx;

use crate::{
    app::shell::{ActiveDocumentChanged, AppShell},
    documents::{DocumentEvent, DocumentPanel, OutlineEntry},
    icons,
    panes::{PaneContent, PaneContext, PaneProvider, PaneSlot},
};

/// The outline's provider.
pub struct OutlinePane;

impl PaneProvider for OutlinePane {
    fn id(&self) -> &'static str {
        "outline"
    }

    fn title(&self) -> SharedString {
        SharedString::from("Outline")
    }

    fn icon(&self) -> lucide_icons::Icon {
        lucide_icons::Icon::ListTree
    }

    fn slot(&self) -> PaneSlot {
        PaneSlot::Left
    }

    fn order(&self) -> i32 {
        10
    }

    fn build(&self, ctx: PaneContext, window: &mut Window, cx: &mut App) -> PaneContent {
        let view = cx.new(|cx| OutlineView::new(ctx.shell, window, cx));
        PaneContent::new(view)
    }
}

/// The outline's state: which document is being shown, and nothing about its
/// contents.
pub struct OutlineView {
    focus_handle: FocusHandle,
    shell: gpui::WeakEntity<AppShell>,
    /// The document the pane is currently outlining, so a re-render can tell
    /// whether the shell moved on.
    document: Option<Entity<DocumentPanel>>,
    /// Dropped with the pane: these are what keep it following the shell.
    _subscriptions: Vec<Subscription>,
}

impl OutlineView {
    fn new(shell: gpui::WeakEntity<AppShell>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // The shell subscription goes on now, but the first read of the shell
        // waits for the end of this frame: the shell builds its panes from
        // inside `update`, so reading it here would re-enter a shell that is
        // already leased.
        let shell_changes = shell.upgrade().map(|shell| {
            cx.subscribe_in(
                &shell,
                window,
                |this: &mut Self, _, _: &ActiveDocumentChanged, window, cx| {
                    this.follow(window, cx);
                },
            )
        });

        let this = Self {
            focus_handle: cx.focus_handle(),
            shell,
            document: None,
            _subscriptions: shell_changes.into_iter().collect(),
        };
        cx.defer_in(window, |this, window, cx| this.follow(window, cx));
        this
    }

    /// Follow whatever the shell is showing.
    ///
    /// Called when the shell says the active document changed and when the
    /// document itself reports its buffer changed — the second is what makes a
    /// heading appear as the user types it.
    fn follow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let shell = self.shell.upgrade();
        let active = shell
            .as_ref()
            .and_then(|shell| shell.read(cx).active_document(cx));

        if active != self.document {
            self.document = active;
            self.resubscribe(window, cx);
        }
        cx.notify();
    }

    /// Point the subscriptions at the shell and the document on show.
    fn resubscribe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self._subscriptions.clear();

        if let Some(shell) = self.shell.upgrade() {
            self._subscriptions.push(cx.subscribe_in(
                &shell,
                window,
                |this: &mut Self, _, _: &ActiveDocumentChanged, window, cx| {
                    this.follow(window, cx);
                },
            ));
        }

        if let Some(document) = self.document.clone() {
            self._subscriptions.push(cx.subscribe_in(
                &document,
                window,
                |_this: &mut Self, _, _: &DocumentEvent, _: &mut Window, cx: &mut Context<Self>| {
                    cx.notify()
                },
            ));
        }
    }

    /// The entries the active viewer reports, or `None` when it does not
    /// outline itself.
    fn entries(&self, cx: &App) -> Option<Vec<OutlineEntry>> {
        let document = self.document.as_ref()?.read(cx);
        document.controller().outline(cx)
    }

    fn jump(&mut self, line: u32, window: &mut Window, cx: &mut Context<Self>) {
        let Some(document) = self.document.clone() else {
            return;
        };
        let controller = document.read(cx).controller().clone();
        controller.goto_line(line, window, cx);
    }
}

impl Focusable for OutlineView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for OutlineView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entries = self.entries(cx);
        let muted = cx.theme().muted_foreground;

        rsx! {
            <div class="flex flex-col size-full" bg={cx.theme().tokens.sidebar}>
                <div
                    class="flex flex-row items-center justify-between gap-1 px-2 py-1"
                    border_b_1
                    border_color={cx.theme().border}
                >
                    <div class="flex flex-row items-center gap-1">
                        {icons::icon(lucide_icons::Icon::ListTree, 14.)}
                        <div class="text-xs">{"Outline"}</div>
                    </div>
                    <Button("outline-refresh")
                        ghost
                        xsmall
                        tooltip={"Refresh"}
                        onClick={cx.listener(|this, _, window, cx| {
                            this.follow(window, cx)
                        })}
                    >
                        {icons::icon(lucide_icons::Icon::RefreshCw, 14.)}
                    </Button>
                </div>
                <div class="flex-1 min-h-0 overflow-hidden">
                    {match entries {
                        None => rsx! {
                            <div class="p-2 text-xs" text_color={muted}>
                                {"This file has no outline."}
                            </div>
                        },
                        Some(entries) if entries.is_empty() => rsx! {
                            <div class="p-2 text-xs" text_color={muted}>
                                {"Nothing to outline in this file."}
                            </div>
                        },
                        Some(entries) => rsx! {
                            <div class="flex flex-col w-full">
                                {for entry in entries {
                                    <div
                                        key={format!("{}:{}", entry.line, entry.label)}
                                        class="flex flex-row items-center w-full h-6 px-2 text-xs overflow-hidden"
                                        pl={px(8. + 12. * entry.depth as f32)}
                                        on_click={cx.listener(move |this, _, window, cx| {
                                            this.jump(entry.line, window, cx)
                                        })}
                                    >
                                        {SharedString::from(entry.label.clone())}
                                    </div>
                                }}
                            </div>
                        },
                    }}
                </div>
            </div>
        }
    }
}
