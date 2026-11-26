//! The text and code viewer.
//!
//! One `EditorState` per document, with the language chosen from the extension.
//! Dirty tracking is an `InputEvent::Change` subscription, and a restore goes
//! through `EditorState::set_value`, which the framework documents as emitting
//! no change event — so a restored tab is not born dirty (risk R10).

use std::{
    path::{Path, PathBuf},
    rc::Rc,
};

use gpui::component::{
    ActiveTheme as _,
    input::{Editor, EditorState, InputEvent},
};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _,
};

use super::{DocumentContent, DocumentController, DocumentProvider, OpenContext, Sniff, sniff};

/// Scores: high enough that nothing else claims a text file by accident, low
/// enough that every specific provider beats it.
const SCORE: u8 = 10;

/// The text viewer.
pub struct TextProvider;

impl DocumentProvider for TextProvider {
    fn id(&self) -> &'static str {
        "text"
    }

    fn title(&self) -> &'static str {
        "Text"
    }

    fn score(&self, path: &Path, sniff: &Sniff) -> Option<u8> {
        // Binary and oversized files are refused outright: the panel would
        // have nothing to show, and the refusal is the honest answer.
        if sniff::text_refusal(sniff).is_some() {
            return None;
        }
        let _ = path;
        Some(SCORE)
    }

    fn open(&self, ctx: OpenContext, window: &mut Window, cx: &mut App) -> DocumentContent {
        let path = ctx.path;
        let language = sniff::language_for(ctx.sniff.ext.as_deref());
        let text = std::fs::read_to_string(&path).unwrap_or_default();

        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .default_value(text)
                .language(language)
                .line_number(true)
        });

        let view = cx.new(|cx| TextView::new(path, language, editor, cx));
        let handle = view.clone();
        let controller: Rc<dyn DocumentController> = Rc::new(TextController { view: handle });
        DocumentContent {
            view: view.into(),
            controller,
        }
    }
}

/// The controller the shell drives.
struct TextController {
    view: Entity<TextView>,
}

impl DocumentController for TextController {
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

    fn language(&self, cx: &App) -> Option<SharedString> {
        Some(SharedString::from(sniff::language_label(
            self.view.read(cx).language,
        )))
    }

    fn cursor(&self, cx: &App) -> Option<(u32, u32)> {
        let position = self.view.read(cx).editor.read(cx).cursor_position();
        Some((position.line + 1, position.character + 1))
    }
}

/// The text viewer's own state: the path, the language, and the editor.
pub struct TextView {
    path: PathBuf,
    language: &'static str,
    editor: Entity<EditorState>,
    focus_handle: FocusHandle,
    /// Whether the buffer differs from the file.
    dirty: bool,
    /// Whether the file changed on disk while the buffer was clean.
    stale: bool,
    /// Dropped with the view, so the subscription does not outlive it.
    _change: Subscription,
}

impl TextView {
    fn new(
        path: PathBuf,
        language: &'static str,
        editor: Entity<EditorState>,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        // Only `Change` marks the buffer dirty: `set_value` suppresses it, so a
        // restore cannot mark a freshly opened tab as modified.
        let change = cx.subscribe(&editor, |this: &mut Self, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.dirty = true;
                cx.notify();
            }
        });

        Self {
            path,
            language,
            editor,
            focus_handle,
            dirty: false,
            stale: false,
            _change: change,
        }
    }

    /// Write the buffer back to disk.
    fn save(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        let text = self.editor.read(cx).value();
        std::fs::write(&self.path, text.as_bytes())
            .map_err(|error| format!("could not save {}: {error}", self.path.display()))?;
        self.dirty = false;
        self.stale = false;
        cx.notify();
        Ok(())
    }

    /// Re-read the file, replacing the buffer.
    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return;
        };
        // `set_value` emits no `Change`, so this cannot re-dirty the buffer.
        self.editor
            .update(cx, |editor, cx| editor.set_value(text, window, cx));
        self.dirty = false;
        self.stale = false;
        cx.notify();
    }

    fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
    }
}

impl Focusable for TextView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<()> for TextView {}

impl Render for TextView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                    .flex_1()
                    .min_h_0()
                    .child(Editor::new(&self.editor).size_full()),
            )
    }
}
