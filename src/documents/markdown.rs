//! The markdown viewer: an editor on the left, a rendered preview on the right,
//! and a toggle between editor-only and split.
//!
//! The preview reuses the editor's text, so there is one buffer and one save
//! path — the same dirty tracking as the text viewer.

use std::{
    path::{Path, PathBuf},
    rc::Rc,
};

use gpui::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    input::{Editor, EditorState, InputEvent},
};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _,
};

use super::{DocumentContent, DocumentController, DocumentProvider, OpenContext, Sniff, sniff};

/// Markdown beats text, and loses to image and video.
const SCORE: u8 = 70;

/// The markdown viewer.
pub struct MarkdownProvider;

impl DocumentProvider for MarkdownProvider {
    fn id(&self) -> &'static str {
        "markdown"
    }

    fn title(&self) -> &'static str {
        "Markdown"
    }

    fn score(&self, _path: &Path, sniff: &Sniff) -> Option<u8> {
        if sniff::text_refusal(sniff).is_some() {
            return None;
        }
        (sniff::kind_of(sniff.ext.as_deref()) == Some(sniff::FileKind::Markdown)).then_some(SCORE)
    }

    fn open(&self, ctx: OpenContext, window: &mut Window, cx: &mut App) -> DocumentContent {
        let path = ctx.path;
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .default_value(text)
                .language("markdown")
                .line_number(true)
        });

        let view = cx.new(|cx| MarkdownView::new(path, editor, cx));
        let controller: Rc<dyn DocumentController> =
            Rc::new(MarkdownController { view: view.clone() });
        DocumentContent {
            view: view.into(),
            controller,
        }
    }
}

struct MarkdownController {
    view: Entity<MarkdownView>,
}

impl DocumentController for MarkdownController {
    fn is_dirty(&self, cx: &App) -> bool {
        self.view.read(cx).dirty
    }

    fn save(&self, window: &mut Window, cx: &mut App) -> Result<(), String> {
        self.view.update(cx, |view, cx| view.save(window, cx))
    }

    fn focus(&self, window: &mut Window, cx: &mut App) {
        self.view.update(cx, |view, cx| view.focus(window, cx));
    }

    fn reload_from_disk(&self, window: &mut Window, cx: &mut App) {
        self.view.update(cx, |view, cx| view.reload(window, cx));
    }

    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.view.read(cx).focus_handle(cx)
    }

    fn is_stale(&self, cx: &App) -> bool {
        self.view.read(cx).stale
    }

    fn accept_disk_change(&self, window: &mut Window, cx: &mut App) {
        self.view.update(cx, |view, cx| {
            view.stale = false;
            view.reload(window, cx);
        });
    }

    fn mark_stale(&self, cx: &mut App) {
        self.view.update(cx, |view, cx| {
            view.stale = true;
            cx.notify();
        });
    }

    fn language(&self, _cx: &App) -> Option<SharedString> {
        Some(SharedString::from("Markdown"))
    }

    fn cursor(&self, cx: &App) -> Option<(u32, u32)> {
        let position = self.view.read(cx).editor.read(cx).cursor_position();
        Some((position.line + 1, position.character + 1))
    }
}

/// How much of the pane the preview takes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Layout {
    /// Editor and preview side by side.
    #[default]
    Split,
    /// Editor only.
    EditorOnly,
    /// Preview only.
    PreviewOnly,
}

/// The markdown viewer's state.
pub struct MarkdownView {
    path: PathBuf,
    editor: Entity<EditorState>,
    focus_handle: FocusHandle,
    dirty: bool,
    stale: bool,
    layout: Layout,
    _change: Subscription,
}

impl MarkdownView {
    fn new(path: PathBuf, editor: Entity<EditorState>, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let change = cx.subscribe(&editor, |this: &mut Self, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.dirty = true;
                cx.notify();
            }
        });
        Self {
            path,
            editor,
            focus_handle,
            dirty: false,
            stale: false,
            layout: Layout::default(),
            _change: change,
        }
    }

    fn save(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        let text = self.editor.read(cx).value();
        std::fs::write(&self.path, text.as_bytes())
            .map_err(|error| format!("could not save {}: {error}", self.path.display()))?;
        self.dirty = false;
        self.stale = false;
        cx.notify();
        Ok(())
    }

    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return;
        };
        self.editor
            .update(cx, |editor, cx| editor.set_value(text, window, cx));
        self.dirty = false;
        self.stale = false;
        cx.notify();
    }

    fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
    }

    fn cycle_layout(&mut self, cx: &mut Context<Self>) {
        self.layout = match self.layout {
            Layout::Split => Layout::EditorOnly,
            Layout::EditorOnly => Layout::PreviewOnly,
            Layout::PreviewOnly => Layout::Split,
        };
        cx.notify();
    }

    fn layout_label(&self) -> &'static str {
        match self.layout {
            Layout::Split => "Split",
            Layout::EditorOnly => "Editor",
            Layout::PreviewOnly => "Preview",
        }
    }
}

impl Focusable for MarkdownView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<()> for MarkdownView {}

impl Render for MarkdownView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let text = self.editor.read(cx).value();
        let show_editor = self.layout != Layout::PreviewOnly;
        let show_preview = self.layout != Layout::EditorOnly;

        div()
            .size_full()
            .flex()
            .flex_col()
            .when(self.stale, |this| {
                this.child(
                    div()
                        .px_2()
                        .py_1()
                        .text_xs()
                        .bg(cx.theme().warning)
                        .text_color(cx.theme().warning_foreground)
                        .child("This file changed on disk."),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_2()
                    .py_1()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Markdown"),
                    )
                    .child(
                        Button::new("markdown-layout")
                            .label(self.layout_label())
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| this.cycle_layout(cx))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .when(show_editor, |this| {
                        this.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Editor::new(&self.editor).size_full()),
                        )
                    })
                    .when(show_editor && show_preview, |this| {
                        this.child(div().w_1().h_full().bg(cx.theme().border).flex_none())
                    })
                    .when(show_preview, |this| {
                        this.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .child(gpui::base::markdown(text).scrollable(true)),
                        )
                    }),
            )
    }
}
