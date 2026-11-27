//! What an editor viewer puts in its status bar.
//!
//! The row itself is the component library's
//! [`StatusBar`](gpui::component::status_bar::StatusBar) — three pinned regions
//! and any `IntoElement`s in them. This file is the seam above it: a viewer
//! that has a buffer and a caret calls [`editor_status_bar`] instead of spelling
//! the same four items again, and gets the file type on the left and the caret
//! and the buffer's size on the right. Actions go on afterwards, with the
//! library's own builder:
//!
//! ```ignore
//! let bar = editor_status_bar(cx, "Markdown", &text, cursor).right(layout_switch);
//! ```
//!
//! The counts are derived from the buffer on every call, so there is no second
//! copy of them to fall out of step, and nothing is cached: one pass over the
//! buffer is the whole cost. What it does *not* do is ask to be redrawn — a
//! viewer whose caret can move without the buffer changing has to re-render
//! itself (`MarkdownView::new` compares the position for exactly that reason).
//!
//! The switches an editor viewer puts in that bar live here too, because every
//! one of them needs the same three things: the glyph, the tint that says which
//! state it is in, and the request for a session write ([`request_session_save`]).

use gpui::component::{
    ActiveTheme as _,
    button::{Button, ButtonVariants as _},
    status_bar::StatusBar,
};
use gpui::{
    App, ClickEvent, IntoElement, ParentElement as _, SharedString, Styled as _, WeakEntity,
    Window, div, px,
};
use gpui_rsx::rsx;

use crate::{app::shell::AppShell, icons};

/// The status bar for `text`, shown as `file_type`.
///
/// `cursor` is 1-based (line, column), the way a reader counts lines; `None`
/// leaves the position out, for a viewer with no caret to report. The file type
/// is drawn in the foreground colour, which is the one thing the library bar
/// does not do: it tints everything in it as secondary text.
pub fn editor_status_bar(
    cx: &App,
    file_type: impl Into<SharedString>,
    text: &str,
    cursor: Option<(u32, u32)>,
) -> StatusBar {
    let file_type = div()
        .text_color(cx.theme().foreground)
        .child(file_type.into());

    let mut bar = StatusBar::new().left(file_type);
    if let Some((line, column)) = cursor {
        bar = bar.right(SharedString::from(format!("Ln {line}, Col {column}")));
    }
    bar.right(SharedString::from(count_label(line_count(text), "line")))
        .right(SharedString::from(count_label(
            character_count(text),
            "character",
        )))
}

/// The status bar's switch for the editor's soft wrap.
///
/// Both states draw the same glyph, so the tint carries the state: wrapped is
/// the foreground colour, unwrapped the bar's own muted one. Only the view can
/// reach its editor, so the click is the caller's.
pub fn wrap_switch(
    cx: &App,
    wrap: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let glyph = icons::icon(lucide_icons::Icon::WrapText, 14.);
    let glyph = if wrap {
        glyph.text_color(cx.theme().foreground)
    } else {
        glyph
    };

    // The square is spelled in `class` rather than with `xsmall`: that variant's
    // padding does not fit a status bar item.
    rsx! {
        <Button("editor-wrap")
            ghost
            class="size-5 p-0"
            tooltip={if wrap { "Line wrap is on" } else { "Line wrap is off" }}
            onClick={on_click}
        >
            {glyph}
        </Button>
    }
}

/// Ask the shell to write the session.
///
/// A viewer's own settings — the wrap switch, the markdown layout — change
/// nothing the shell is watching for, and the quit hook alone is not enough
/// because anything that kills the app never reaches it. Every switch that a
/// `view_state` restores calls this, or it is only saved by the next unrelated
/// write.
pub fn request_session_save(shell: &WeakEntity<AppShell>, window: &mut Window, cx: &mut App) {
    // A shell that is gone has nothing to write; the change itself still stands.
    let _ = shell.update(cx, |shell, cx| shell.schedule_session_save(window, cx));
}

/// How many lines the editor draws for `text`.
///
/// The count is the `\n`s plus one, so a buffer ending in a newline has the
/// empty line the editor shows for it rather than one fewer.
fn line_count(text: &str) -> usize {
    text.split('\n').count()
}

/// The characters in `text`, excluding the line breaks.
///
/// The breaks are already counted by [`line_count`] and are not characters a
/// reader counts: the two numbers are meant to be read side by side, and a CRLF
/// counted as two would make them disagree with what the editor shows.
fn character_count(text: &str) -> usize {
    text.chars()
        .filter(|character| !matches!(character, '\n' | '\r'))
        .count()
}

/// `1 line` and `3 lines`.
fn count_label(count: usize, unit: &str) -> String {
    let plural = if count == 1 { "" } else { "s" };
    format!("{} {unit}{plural}", group_digits(count))
}

/// `1234567` as `1,234,567`, so a large count can be read at a glance.
fn group_digits(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_buffer_is_one_line() {
        assert_eq!(line_count(""), 1);
        assert_eq!(line_count("a"), 1);
        // The editor draws the empty line after a trailing newline.
        assert_eq!(line_count("a\n"), 2);
        assert_eq!(line_count("a\r\nb"), 2);
    }

    #[test]
    fn characters_exclude_the_line_breaks() {
        assert_eq!(character_count(""), 0);
        assert_eq!(character_count("a\r\nb"), 2);
        // Counted in characters, not bytes.
        assert_eq!(character_count("héllo"), 5);
    }

    #[test]
    fn counts_read_as_english() {
        assert_eq!(count_label(0, "line"), "0 lines");
        assert_eq!(count_label(1, "line"), "1 line");
        assert_eq!(count_label(2048, "character"), "2,048 characters");
    }

    #[test]
    fn large_numbers_get_a_separator_every_three_digits() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1000), "1,000");
        assert_eq!(group_digits(1234567), "1,234,567");
    }
}
