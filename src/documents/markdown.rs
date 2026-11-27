//! The markdown viewer: an editor, a rendered preview, and a switch between the
//! two layouts. The preview reuses the editor's text, so there is one buffer and
//! one save path.

use std::{
    path::{Path, PathBuf},
    rc::Rc,
};

use gpui::component::{
    ActiveTheme as _,
    button::{Button, ButtonVariants as _},
    input::{Editor, EditorState, InputEvent, Position},
};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, WeakEntity, Window, div, px,
};
use gpui_rsx::rsx;

use super::{
    DocumentContent, DocumentController, DocumentEvent, DocumentProvider, OpenContext, Sniff,
    sniff,
    status_bar_helper::{editor_status_bar, request_session_save, wrap_switch},
};
use crate::{app::shell::AppShell, icons};

/// Markdown beats text, and loses to image.
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

        let view = cx.new(|cx| MarkdownView::new(path, editor, ctx.shell, cx));
        let controller: Rc<dyn DocumentController> =
            Rc::new(MarkdownController { view: view.clone() });
        let watched = view.clone();
        DocumentContent {
            view: view.into(),
            controller,
            watch: Some(Box::new(move |cx| {
                let watched = watched.clone();
                cx.subscribe(&watched, |_, _, _: &(), cx| {
                    cx.emit(DocumentEvent);
                    cx.notify();
                })
            })),
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

    fn view_state(&self, cx: &App) -> Option<super::ViewState> {
        let view = self.view.read(cx);
        match serde_json::to_value(MarkdownViewState {
            layout: view.layout,
            wrap: Some(view.wrap),
            cursor: Some(view.editor.read(cx).cursor_position()),
        }) {
            Ok(state) => Some(state),
            Err(error) => {
                log::warn!("could not encode the markdown view state: {error}");
                None
            }
        }
    }

    fn restore_view_state(&self, state: &super::ViewState, window: &mut Window, cx: &mut App) {
        // A value this build does not understand leaves the defaults in place.
        let Ok(state) = serde_json::from_value::<MarkdownViewState>(state.clone()) else {
            return;
        };
        self.view.update(cx, |view, cx| {
            view.layout = state.layout;
            view.wrap = state.wrap.unwrap_or(true);
            view.editor.update(cx, |editor, cx| {
                editor.set_soft_wrap(view.wrap, window, cx);
                // Moving the caret is not an edit, so a restored tab stays clean.
                if let Some(position) = state.cursor {
                    editor.set_cursor_position(position, window, cx);
                }
            });
            view.last_cursor = view.editor.read(cx).cursor_position();
            cx.notify();
        });
    }

    fn cursor(&self, cx: &App) -> Option<(u32, u32)> {
        let position = self.view.read(cx).editor.read(cx).cursor_position();
        Some((position.line + 1, position.character + 1))
    }

    fn outline(&self, cx: &App) -> Option<Vec<super::OutlineEntry>> {
        // Read from the editor, not the file, so unsaved headings show up.
        let text = self.view.read(cx).editor.read(cx).value();
        Some(super::markdown_outline::headings(&text))
    }

    fn goto_line(&self, line: u32, window: &mut Window, cx: &mut App) {
        self.view.update(cx, |view, cx| {
            view.editor.update(cx, |editor, cx| {
                editor.set_cursor_position(
                    gpui::component::input::Position::new(line.saturating_sub(1), 0),
                    window,
                    cx,
                );
            });
        });
    }
}

/// How much of the pane the preview takes. The names are written to the session
/// file, so renaming one loses the layout of documents saved under the old name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Layout {
    #[serde(rename = "split")]
    Split,
    #[default]
    #[serde(rename = "editor")]
    EditorOnly,
    #[serde(rename = "preview")]
    PreviewOnly,
}

/// What the markdown viewer keeps between runs. Optional fields are what an older
/// session leaves out; each falls back to a fresh tab's default.
#[derive(serde::Serialize, serde::Deserialize)]
struct MarkdownViewState {
    layout: Layout,
    #[serde(default)]
    wrap: Option<bool>,
    #[serde(default)]
    cursor: Option<Position>,
}

impl Layout {
    /// A pen for the editor, columns for the split, an eye for the preview.
    fn icon(self) -> lucide_icons::Icon {
        match self {
            Layout::Split => lucide_icons::Icon::Columns2,
            Layout::EditorOnly => lucide_icons::Icon::SquarePen,
            Layout::PreviewOnly => lucide_icons::Icon::Eye,
        }
    }
}

/// The markdown viewer's state.
pub struct MarkdownView {
    path: PathBuf,
    editor: Entity<EditorState>,
    focus_handle: FocusHandle,
    dirty: bool,
    stale: bool,
    layout: Layout,
    wrap: bool,
    /// The wrap and layout choices are written to the session by the shell.
    shell: WeakEntity<AppShell>,
    /// The caret position the status bar was last drawn for.
    last_cursor: Position,
    _change: Subscription,
    _cursor: Subscription,
}

impl MarkdownView {
    fn new(
        path: PathBuf,
        editor: Entity<EditorState>,
        shell: WeakEntity<AppShell>,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        let change = cx.subscribe(&editor, |this: &mut Self, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.dirty = true;
                // The panel re-broadcasts this to panes derived from the buffer.
                cx.emit(());
                cx.notify();
            }
        });
        // The editor notifies on its blink cycle too, so redraw only on a move —
        // otherwise the preview would re-render twice a second.
        let last_cursor = editor.read(cx).cursor_position();
        let cursor = cx.observe(&editor, |this: &mut Self, editor, cx| {
            let position = editor.read(cx).cursor_position();
            if position != this.last_cursor {
                this.last_cursor = position;
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
            wrap: true,
            shell,
            last_cursor,
            _change: change,
            _cursor: cursor,
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
        self.dirty = false;
        self.stale = false;
        // The watcher reports our own save as a disk change; resetting the buffer
        // to its own text would drop the caret and the scroll on every save.
        if self.editor.read(cx).value().as_ref() == text {
            cx.notify();
            return;
        }
        self.editor
            .update(cx, |editor, cx| editor.set_value(text, window, cx));
        cx.notify();
    }

    fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
    }

    /// Step to the next layout: split, editor only, preview only, and around.
    fn cycle_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.layout = match self.layout {
            Layout::Split => Layout::EditorOnly,
            Layout::EditorOnly => Layout::PreviewOnly,
            Layout::PreviewOnly => Layout::Split,
        };
        cx.notify();
        request_session_save(&self.shell, window, cx);
    }

    /// Turn soft wrap on or off, and save the choice with the tab.
    fn toggle_wrap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.wrap = !self.wrap;
        let wrap = self.wrap;
        self.editor
            .update(cx, |editor, cx| editor.set_soft_wrap(wrap, window, cx));
        cx.notify();
        request_session_save(&self.shell, window, cx);
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
        let stale = self.stale;
        let warning = cx.theme().warning;
        let warning_foreground = cx.theme().warning_foreground;
        let border = cx.theme().border;

        let position = self.editor.read(cx).cursor_position();
        // The lucide font is not an SVG, so the glyph is a child rather than
        // `Button::icon`; `class` sets the square because `xsmall` adds padding.
        let layout_switch = rsx! {
            <Button("markdown-layout")
                ghost
                class="size-5 p-0"
                tooltip={self.layout_label()}
                onClick={cx.listener(|this, _, window, cx| this.cycle_layout(window, cx))}
            >
                {icons::icon(self.layout.icon(), 14.)}
            </Button>
        };
        let status = editor_status_bar(
            cx,
            "Markdown",
            &text,
            Some((position.line + 1, position.character + 1)),
        )
        .right(wrap_switch(
            cx,
            self.wrap,
            cx.listener(|this, _, window, cx| this.toggle_wrap(window, cx)),
        ))
        .right(layout_switch);

        rsx! {
            <div
                class="flex flex-col size-full"
                key_context={super::text::DOCUMENT_CONTEXT}
                onAction={cx.listener(|this, _: &super::text::SaveDocument, window, cx| {
                    if let Err(error) = this.save(window, cx) {
                        log::error!("{error}");
                    }
                })}
            >
                {if stale {
                    rsx! {
                        <div
                            class="px-2 py-1 text-xs"
                            bg={warning}
                            text_color={warning_foreground}
                        >
                            {"This file changed on disk."}
                        </div>
                    }
                } else {
                    rsx! { <div /> }
                }}
                <div class="flex flex-row flex-1 min-h-0">
                    {if show_editor {
                        rsx! {
                            <div class="flex-1 min-w-0">
                                {Editor::new(&self.editor).size_full().bordered(false)}
                            </div>
                        }
                    } else {
                        rsx! { <div /> }
                    }}
                    {if show_editor && show_preview {
                        rsx! { <div w={px(1.)} h_full flex_none bg={border} /> }
                    } else {
                        rsx! { <div /> }
                    }}
                    {if show_preview {
                        rsx! {
                            <div class="flex-1 min-w-0 overflow-hidden">
                                <gpui::base::markdown(text) scrollable={true} class="px-4" />
                            </div>
                        }
                    } else {
                        rsx! { <div /> }
                    }}
                </div>
                {status}
            </div>
        }
    }
}
