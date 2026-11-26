//! The title bar.
//!
//! `gpui_component::TitleBar` already owns the window drag, the double-click
//! zoom and the native window controls, so this only supplies the app mark on
//! the left and the folder actions on the right.

use gpui::component::{
    ActiveTheme as _, Sizable as _, TitleBar,
    button::{Button, ButtonVariants as _},
};
use gpui::{
    AnyElement, App, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Styled as _, Window, div, px,
};
use gpui_rsx::rsx;

use crate::icons;

/// The app mark: a small square in the accent colour, then the name.
fn app_mark(accent: gpui::Hsla) -> impl IntoElement {
    rsx! {
        <div class="flex items-center gap-2">
            <div w={px(12.)} h={px(12.)} rounded={px(3.)} bg={accent} />
            <div class="text-sm">{"lllearning"}</div>
        </div>
    }
}

/// The title bar, with the folder actions the shell wires up.
///
/// Returns an owned element rather than an `impl IntoElement`, so the caller can
/// keep building its own tree with `cx` after this returns.
pub fn render(
    on_pick: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    on_refresh: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let accent = cx.theme().primary;
    let mark = app_mark(accent);

    rsx! {
        <TitleBar base={TitleBar::new()}>
            {mark}
            <div class="flex items-center gap-1">
                <Button("title-open-folder")
                    ghost
                    xsmall
                    tooltip={"Change folder…"}
                    onClick={on_pick}
                >
                    {icons::icon(lucide_icons::Icon::FolderOpen, 14.)}
                </Button>
                <Button("title-refresh")
                    ghost
                    xsmall
                    tooltip={"Refresh the tree"}
                    onClick={on_refresh}
                >
                    {icons::icon(lucide_icons::Icon::RefreshCw, 14.)}
                </Button>
            </div>
        </TitleBar>
    }
    .into_any_element()
}

/// The title bar with no actions, for a window that has nothing to act on.
pub fn render_plain(cx: &App) -> AnyElement {
    let mark = app_mark(cx.theme().primary);
    rsx! {
        <TitleBar base={TitleBar::new()}>
            {mark}
        </TitleBar>
    }
    .into_any_element()
}

/// Keeps the `SharedString` import honest until a title needs one.
#[allow(dead_code)]
fn label(text: &str) -> SharedString {
    SharedString::from(text.to_owned())
}
