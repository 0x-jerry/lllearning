//! What an editor viewer puts in its status bar: the file type on the left, the
//! caret and the buffer's counts on the right, and the switches (wrap, layout)
//! every editor viewer needs. The row itself is the component library's
//! [`StatusBar`].
//!
//! The counts come from the buffer on every call, so they cannot drift; the bar
//! does not ask to be redrawn, so a viewer whose caret moves without the buffer
//! changing has to re-render itself.

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

/// The status bar for `text`, shown as `file_type`. `cursor` is 1-based.
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

/// The status bar's switch for the editor's soft wrap: both states share the
/// glyph, so the tint carries the state.
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

    // `class` rather than `xsmall`: that variant's padding does not fit a status bar item.
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

/// Ask the shell to write the session. A switch's setting changes nothing the
/// shell is watching for, and the quit hook alone is not enough because a killed
/// process never reaches it.
pub fn request_session_save(shell: &WeakEntity<AppShell>, window: &mut Window, cx: &mut App) {
    // A shell that is gone has nothing to write; the change itself still stands.
    let _ = shell.update(cx, |shell, cx| shell.schedule_session_save(window, cx));
}

/// How many lines the editor draws for `text`: the `\n`s plus one, so a buffer
/// ending in a newline has the empty line the editor shows.
fn line_count(text: &str) -> usize {
    text.split('\n').count()
}

/// The characters in `text`, excluding the line breaks, which are already counted
/// by [`line_count`].
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
