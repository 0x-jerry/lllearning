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
    ActiveTheme as _, Sizable as _,
    button::Button,
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
use crate::app::shell::AppShell;

/// Scores: high enough that nothing else claims a text file by accident, low
/// enough that every specific provider beats it.
const SCORE: u8 = 10;

/// The key context the editor panes put on their root element.
///
/// Bindings scoped to this context reach a document viewer and nothing else, so
/// `ctrl-s` cannot be swallowed by an unrelated input elsewhere in the window.
pub const DOCUMENT_CONTEXT: &str = "DocumentViewer";

gpui::actions!(lllearning, [SaveDocument]);

/// Bind `ctrl-s` (and `cmd-s` on macOS) to [`SaveDocument`].
///
/// `ctrl-s` is unbound everywhere in the framework, so claiming it here does not
/// shadow anything. The binding is registered once, at startup.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        gpui::KeyBinding::new("ctrl-s", SaveDocument, Some(DOCUMENT_CONTEXT)),
        gpui::KeyBinding::new("cmd-s", SaveDocument, Some(DOCUMENT_CONTEXT)),
    ]);
}

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
        // Binary and oversized files are refused by this provider: the panel
        // would have nothing to show, and the refusal is the honest answer.
        // They still open — `RefusedProvider` claims them below and explains
        // why.
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

        let view = cx.new(|cx| TextView::new(path, language, editor, ctx.shell, cx));
        let handle = view.clone();
        let controller: Rc<dyn DocumentController> = Rc::new(TextController { view: handle });
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

    fn mark_stale(&self, cx: &mut App) {
        self.view.update(cx, |view, cx| {
            view.stale = true;
            cx.notify();
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

    fn view_state(&self, cx: &App) -> Option<super::ViewState> {
        // A state that cannot be encoded is a wiring bug, not something the
        // user did: log it and let the tab reopen at its defaults next time.
        let view = self.view.read(cx);
        match serde_json::to_value(TextViewState {
            wrap: Some(view.wrap),
            cursor: Some(view.editor.read(cx).cursor_position()),
        }) {
            Ok(state) => Some(state),
            Err(error) => {
                log::warn!("could not encode the text view state: {error}");
                None
            }
        }
    }

    fn restore_view_state(&self, state: &super::ViewState, window: &mut Window, cx: &mut App) {
        // A value this build does not understand leaves the default in place
        // rather than failing the open (risk R12).
        let Ok(state) = serde_json::from_value::<TextViewState>(state.clone()) else {
            return;
        };
        self.view.update(cx, |view, cx| {
            view.wrap = state.wrap.unwrap_or(true);
            view.editor.update(cx, |editor, cx| {
                editor.set_soft_wrap(view.wrap, window, cx);
                // Moving the caret is not an edit: `set_cursor_position` emits
                // no change, so a restored tab is not born dirty (risk R10).
                // It also focuses the editor, which the shell corrects by
                // focusing the active tab once every document is restored.
                if let Some(position) = state.cursor {
                    editor.set_cursor_position(position, window, cx);
                }
            });
            // The caret moved under the status bar's feet, so the position it
            // redraws from is taken again rather than left behind.
            view.last_cursor = view.editor.read(cx).cursor_position();
            cx.notify();
        });
    }
}

/// What the text viewer keeps between runs.
///
/// Its own type, so the session file's shape is spelled here rather than in the
/// shell. The optional fields are what a session written before they existed
/// leaves out; each then falls back to what a fresh tab would do.
#[derive(serde::Serialize, serde::Deserialize)]
struct TextViewState {
    /// Soft wrap, as the status bar's switch left it.
    #[serde(default)]
    wrap: Option<bool>,
    /// Where the caret was, so a long file reopens where it was left.
    #[serde(default)]
    cursor: Option<Position>,
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
    /// The caret position the status bar was last drawn for, so a notify that
    /// does not move the caret does not redraw the view.
    last_cursor: Position,
    /// Whether the editor wraps long lines, as the status bar's switch leaves it.
    wrap: bool,
    /// The shell, because the wrap choice is part of the session and only the
    /// shell writes that file (see [`Self::toggle_wrap`]).
    shell: WeakEntity<AppShell>,
    /// Dropped with the view, so the subscription does not outlive it.
    _change: Subscription,
    _cursor: Subscription,
}

impl TextView {
    fn new(
        path: PathBuf,
        language: &'static str,
        editor: Entity<EditorState>,
        shell: WeakEntity<AppShell>,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        // Only `Change` marks the buffer dirty: `set_value` suppresses it, so a
        // restore cannot mark a freshly opened tab as modified.
        let change = cx.subscribe(&editor, |this: &mut Self, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.dirty = true;
                // The panel re-broadcasts this, so a pane showing something
                // derived from the buffer (the outline) follows the typing.
                cx.emit(());
                cx.notify();
            }
        });

        let last_cursor = editor.read(cx).cursor_position();
        // The status bar shows the caret, and the editor emits no event for a
        // move that changes no text. It does notify, but also for things that
        // leave the caret where it is (its blink cycle runs off its own
        // entity), so this redraws only when the position actually moved.
        let cursor = cx.observe(&editor, |this: &mut Self, editor, cx| {
            let position = editor.read(cx).cursor_position();
            if position != this.last_cursor {
                this.last_cursor = position;
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
            last_cursor,
            wrap: true,
            shell,
            _change: change,
            _cursor: cursor,
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
        self.dirty = false;
        self.stale = false;
        // Saving writes the buffer out, and the watcher reports that write as a
        // disk change; replacing the buffer with its own text would reset the
        // caret and the scroll on every save.
        if self.editor.read(cx).value().as_ref() == text {
            cx.notify();
            return;
        }
        // `set_value` emits no `Change`, so this cannot re-dirty the buffer.
        self.editor
            .update(cx, |editor, cx| editor.set_value(text, window, cx));
        cx.notify();
    }

    fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
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
}

impl Focusable for TextView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<()> for TextView {}

impl Render for TextView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let warning = cx.theme().warning;
        let warning_foreground = cx.theme().warning_foreground;
        let stale = self.stale;

        let text = self.editor.read(cx).value();
        let position = self.editor.read(cx).cursor_position();
        let status = editor_status_bar(
            cx,
            sniff::language_label(self.language),
            &text,
            Some((position.line + 1, position.character + 1)),
        )
        .right(wrap_switch(
            cx,
            self.wrap,
            cx.listener(|this, _, window, cx| this.toggle_wrap(window, cx)),
        ));

        rsx! {
            <div
                class="flex flex-col size-full"
                key_context={DOCUMENT_CONTEXT}
                onAction={cx.listener(|this, _: &SaveDocument, window, cx| {
                    // A save that fails is reported rather than swallowed: the
                    // tab must not lose its dirty marker on a failed write.
                    if let Err(error) = this.save(window, cx) {
                        log::error!("{error}");
                    }
                })}
            >
                {if stale {
                    rsx! {
                        <div
                            class="flex flex-row items-center justify-between gap-2 px-2 py-1 text-xs"
                            bg={warning}
                            text_color={warning_foreground}
                        >
                            <div>{"This file changed on disk."}</div>
                            <Button("text-reload")
                                xsmall
                                label={"Reload"}
                                onClick={cx.listener(|this, _, window, cx| this.reload(window, cx))}
                            />
                        </div>
                    }
                } else {
                    rsx! { <div /> }
                }}
                <div class="flex-1 min-h-0">
                    {Editor::new(&self.editor).size_full().bordered(false)}
                </div>
                {status}
            </div>
        }
    }
}
