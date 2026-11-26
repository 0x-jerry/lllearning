//! The outline pane: the headings of the active markdown document, or the
//! symbols of the active code file.
//!
//! This pane exists to prove the seam: it is a new module plus one
//! `register_pane` line, and nothing in `app/shell.rs` or `app/skin.rs` changed to
//! accommodate it. It reads the active document through the shell and extracts
//! its outline with plain text scanning — no parser, because an outline that
//! costs a tree-sitter parse per keystroke is not worth it for a first pass.

use std::sync::Arc;

use gpui::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
};
use gpui::{
    App, AppContext as _, Context, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Window, div, px,
};
use gpui_rsx::rsx;

use crate::{
    app::shell::AppShell,
    icons,
    panes::{PaneContext, PaneProvider, PaneSlot},
};

/// One line of the outline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineEntry {
    /// The text, without the syntax that produced it.
    pub label: String,
    /// 0 for a top-level heading, 1 for a nested one, and so on.
    pub depth: usize,
}

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

    fn build(
        &self,
        ctx: PaneContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Arc<dyn gpui::component::dock::BasePanelView> {
        let _ = ctx.saved;
        let view = cx.new(|cx| OutlineView::new(ctx.shell, window, cx));
        gpui::component::dock::panel_handle(view)
    }
}

/// The outline's state.
pub struct OutlineView {
    focus_handle: FocusHandle,
    shell: gpui::WeakEntity<AppShell>,
    entries: Vec<OutlineEntry>,
    /// What the entries were extracted from, so a re-render can tell whether
    /// anything changed.
    source: String,
}

impl OutlineView {
    fn new(
        shell: gpui::WeakEntity<AppShell>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            shell,
            entries: Vec::new(),
            source: String::new(),
        }
    }

    /// Re-read the active document's text and rebuild the outline.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.active_document_text(cx) else {
            self.entries.clear();
            self.source.clear();
            cx.notify();
            return;
        };
        if text == self.source {
            return;
        }
        self.entries = extract_outline(&text);
        self.source = text;
        cx.notify();
    }

    /// The active document's text, through the shell.
    fn active_document_text(&self, cx: &mut Context<Self>) -> Option<String> {
        let shell = self.shell.upgrade()?;
        let shell = shell.read(cx);
        let document = shell.active_document()?;
        let document = document.read(cx);
        // The controller is the only thing the shell exposes about a document's
        // contents, so the text is read back from the file it is showing.
        std::fs::read_to_string(document.path()).ok()
    }
}

impl Focusable for OutlineView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<gpui::base::dock::PanelEvent> for OutlineView {}

impl gpui::base::dock::Panel for OutlineView {
    fn panel_name(&self) -> &'static str {
        "outline"
    }

    fn dump(&self, _cx: &App) -> gpui::component::dock::PanelState {
        gpui::component::dock::PanelState::new("outline")
    }
}

impl gpui::component::dock::Panel for OutlineView {
    fn tab_name(&self, _cx: &App) -> Option<SharedString> {
        Some(SharedString::from("Outline"))
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        SharedString::from("Outline")
    }

    fn inner_padding(&self, _cx: &App) -> bool {
        false
    }

    fn title_bar(&self, _cx: &App) -> bool {
        false
    }
}

impl Render for OutlineView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Reading the file each frame is cheap relative to a parse and keeps the
        // outline live without a change subscription the shell does not expose.
        let text = self.active_document_text(cx);
        let entries = match &text {
            Some(text) if text != &self.source => {
                self.source = text.clone();
                self.entries = extract_outline(text);
                self.entries.clone()
            }
            _ => self.entries.clone(),
        };
        let muted = cx.theme().muted_foreground;
        let empty = entries.is_empty();

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
                        onClick={cx.listener(|this, _, _, cx| this.refresh(cx))}
                    >
                        {icons::icon(lucide_icons::Icon::RefreshCw, 14.)}
                    </Button>
                </div>
                <div class="flex-1 min-h-0 overflow-hidden">
                    {if empty {
                        rsx! {
                            <div class="p-2 text-xs" text_color={muted}>
                                {"No headings or symbols in the active file."}
                            </div>
                        }
                    } else {
                        rsx! {
                            <div class="flex flex-col w-full">
                                {for entry in entries {
                                    <div
                                        key={format!("{}-{}", entry.depth, entry.label)}
                                        class="flex flex-row items-center w-full h-6 px-2 text-xs overflow-hidden"
                                        pl={px(8. + 12. * entry.depth as f32)}
                                    >
                                        {SharedString::from(entry.label.clone())}
                                    </div>
                                }}
                            </div>
                        }
                    }}
                </div>
            </div>
        }
    }
}

/// Extract an outline from `text`.
///
/// Markdown headings (`#`, `##`, …) and common code declarations are recognised.
/// Anything else contributes nothing, so a plain text file has an empty outline
/// rather than a list of every line.
pub fn extract_outline(text: &str) -> Vec<OutlineEntry> {
    let mut entries = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(entry) = markdown_heading(trimmed) {
            entries.push(entry);
            continue;
        }
        if let Some(entry) = code_symbol(trimmed) {
            entries.push(entry);
        }
    }
    entries
}

/// A markdown ATX heading.
fn markdown_heading(line: &str) -> Option<OutlineEntry> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = line[hashes..].strip_prefix(' ')?;
    let label = rest.trim_end_matches('#').trim();
    if label.is_empty() {
        return None;
    }
    Some(OutlineEntry {
        label: label.to_owned(),
        depth: hashes - 1,
    })
}

/// A function, struct, class or type declaration at the start of a line.
fn code_symbol(line: &str) -> Option<OutlineEntry> {
    const PREFIXES: [&str; 10] = [
        "pub fn ",
        "fn ",
        "pub struct ",
        "struct ",
        "pub enum ",
        "enum ",
        "pub trait ",
        "trait ",
        "class ",
        "def ",
    ];
    for prefix in PREFIXES {
        if let Some(rest) = line.strip_prefix(prefix) {
            let name = rest
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .next()
                .unwrap_or("");
            if !name.is_empty() {
                return Some(OutlineEntry {
                    label: name.to_owned(),
                    depth: 0,
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_headings_become_nested_entries() {
        let text = "# Title\n\nSome prose.\n\n## Section\n\n### Deeper\n";
        let entries = extract_outline(text);
        assert_eq!(
            entries,
            vec![
                OutlineEntry {
                    label: "Title".to_owned(),
                    depth: 0
                },
                OutlineEntry {
                    label: "Section".to_owned(),
                    depth: 1
                },
                OutlineEntry {
                    label: "Deeper".to_owned(),
                    depth: 2
                },
            ]
        );
    }

    #[test]
    fn a_heading_without_text_is_ignored() {
        assert!(extract_outline("#\n##   \n").is_empty());
    }

    #[test]
    fn more_than_six_hashes_is_not_a_heading() {
        assert!(extract_outline("####### Too deep\n").is_empty());
    }

    #[test]
    fn code_declarations_become_entries() {
        let text = "pub fn open() {}\nstruct Thing;\nfn private() {}\n";
        let entries = extract_outline(text);
        let labels: Vec<&str> = entries.iter().map(|entry| entry.label.as_str()).collect();
        assert_eq!(labels, vec!["open", "Thing", "private"]);
    }

    #[test]
    fn plain_prose_has_no_outline() {
        assert!(extract_outline("just some words\nand more words\n").is_empty());
    }

    #[test]
    fn indented_declarations_are_recognised() {
        let entries = extract_outline("    fn nested() {}\n");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].label, "nested");
    }
}
