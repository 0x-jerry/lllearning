//! The footer.
//!
//! Small, muted, and one pixel of border — per the mockup. It shows the
//! workspace folder, the active file and its language, the caret position when
//! the viewer reports one, and whether the tree is being watched.

use gpui::component::{ActiveTheme as _, status_bar::StatusBar};
use gpui::{AnyElement, App, IntoElement, ParentElement as _, SharedString, Styled as _, div, px};
use gpui_rsx::rsx;
use lucide_icons::Icon;

use crate::{app::shell::AppShell, icons, workspace::fs_util};

/// The footer as an element.
///
/// Returns an owned element so the caller keeps `cx` free for its own tree.
pub fn render(shell: &AppShell, cx: &App) -> AnyElement {
    let root = shell.workspace().read(cx).root().map(|path| {
        rsx! {
            <div class="flex flex-row items-center gap-1 min-w-0">
                {icons::icon(Icon::Book, 12.)}
                <div class="truncate">{fs_util::file_name_of(path)}</div>
            </div>
        }
        .into_any_element()
    });
    let watching = shell.workspace().read(cx).is_watching();
    let dirty_count = shell
        .documents()
        .iter()
        .filter(|document| document.read(cx).is_dirty(cx))
        .count();

    let mut bar = StatusBar::new().left(match root {
        Some(root) => root,
        None => SharedString::from("No folder open").into_any_element(),
    });

    if let Some(document) = shell.active_document(cx) {
        let document = document.read(cx);
        bar = bar
            .left(separator(cx))
            .left(SharedString::from(fs_util::file_name_of(document.path())));
        // The viewer's own name for what it is, so a refused file reads
        // "Unsupported" rather than claiming a language it does not have.
        if let Some(provider) = crate::documents::provider_by_id(cx, document.provider_id()) {
            bar = bar
                .left(separator(cx))
                .left(SharedString::from(provider.title()));
        }
        if let Some(language) = document.controller().language(cx) {
            bar = bar.left(separator(cx)).left(language);
        }
        if let Some((line, column)) = document.controller().cursor(cx) {
            bar = bar
                .left(separator(cx))
                .left(SharedString::from(format!("Ln {line}, Col {column}")));
        }
        if document.is_dirty(cx) {
            bar = bar.left(separator(cx)).left(SharedString::from("Unsaved"));
        }
    }

    if dirty_count > 0 {
        bar = bar.right(SharedString::from(format!("{dirty_count} unsaved")));
    }

    bar.right(SharedString::from(if watching {
        "Watching"
    } else {
        "Not watching"
    }))
    .into_any_element()
}

/// The hairline between footer items.
fn separator(cx: &App) -> AnyElement {
    let border = cx.theme().border;
    rsx! { <div w={px(1.)} h={px(10.)} flex_none bg={border} /> }.into_any_element()
}
