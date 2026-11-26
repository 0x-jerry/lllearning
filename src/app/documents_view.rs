//! The centre: the document tabs and the document on show.
//!
//! The strip and the view are one thing because a tab and the document it names
//! are the same index — a close button that had to look its document up by path
//! would be a second source of truth for "which tab is this".
//!
//! Which tab is on show lives here rather than in the shell: nothing outside the
//! centre selects a tab, and the shell only needs to know which document is
//! active, which it asks for.

use std::cell::Cell;

use gpui::component::{
    ActiveTheme as _, Selectable as _, Sizable as _,
    button::{Button, ButtonCustomVariant, ButtonVariants as _},
    tab::{Tab, TabBar},
};
use gpui::{
    AnyElement, App, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    ScrollHandle, SharedString, Styled as _, Window, div, px,
};
use gpui_rsx::rsx;

use crate::{
    app::shell::{empty_state, frame},
    documents::DocumentPanel,
};

/// The centre of the window.
pub struct DocumentsView {
    /// Every document open, in tab order.
    documents: Vec<Entity<DocumentPanel>>,
    /// The index in `documents` the centre is showing.
    active: Option<usize>,
    /// The strip's scroll position.
    scroll: ScrollHandle,
    /// The tab the strip last scrolled to. Kept so the strip moves when the
    /// selection changes and not on every frame -- otherwise scrolling away from
    /// the active tab would be undone by the next render.
    shown: Cell<Option<usize>>,
}

impl Default for DocumentsView {
    fn default() -> Self {
        Self {
            documents: Vec::new(),
            active: None,
            scroll: ScrollHandle::new(),
            shown: Cell::new(None),
        }
    }
}

impl DocumentsView {
    /// Every document open, in tab order.
    pub fn documents(&self) -> &[Entity<DocumentPanel>] {
        &self.documents
    }

    /// The document on show.
    pub fn active(&self) -> Option<Entity<DocumentPanel>> {
        self.documents.get(self.active?).cloned()
    }

    /// The tab on show, for the session file.
    pub fn saved_active(&self) -> Option<usize> {
        self.active
    }

    /// Show the document at `ix`.
    pub fn select(&mut self, ix: usize) -> bool {
        if ix >= self.documents.len() || self.active == Some(ix) {
            return false;
        }
        self.active = Some(ix);
        true
    }

    /// Append a document as the new active tab.
    pub fn push(&mut self, document: Entity<DocumentPanel>) {
        self.documents.push(document);
        self.active = Some(self.documents.len() - 1);
    }

    /// Forget every document.
    pub fn clear(&mut self) {
        self.documents.clear();
        self.active = None;
    }

    /// Set the active tab from the session, falling back to the first.
    pub fn restore_active(&mut self, saved: Option<usize>) {
        self.active = saved
            .filter(|ix| *ix < self.documents.len())
            .or_else(|| (!self.documents.is_empty()).then_some(0));
    }

    /// Drop a document's tab, and pick the tab that replaces it.
    ///
    /// Returns the document that is now on show, so the caller can focus it and
    /// bring it into view in the tree.
    pub fn remove(&mut self, document: &Entity<DocumentPanel>) -> Option<Entity<DocumentPanel>> {
        let ix = self.documents.iter().position(|open| open == document)?;
        self.documents.remove(ix);
        self.active = next_active(ix, self.documents.len());
        self.active()
    }

    /// The centre, framed with its tab strip.
    ///
    /// `on_select` and `on_close` are the shell's: selecting focuses the editor
    /// and reveals the file in the tree, and closing may have to prompt for
    /// unsaved work, so neither belongs to a view that knows no shell.
    pub fn render(
        &self,
        on_select: impl Fn(&usize, &mut Window, &mut App) + 'static,
        on_close: impl Fn(&usize, &mut Window, &mut App) + 'static,
        cx: &App,
    ) -> AnyElement {
        let active = self.active.unwrap_or(0);
        let body = match self.active() {
            Some(document) => document.into_any_element(),
            None => empty_state("No file open", "Pick a file in the explorer to start.", cx),
        };

        let strip =
            (!self.documents.is_empty()).then(|| self.tab_strip(active, on_select, on_close, cx));

        frame("pane-documents", strip, body, cx)
    }

    fn tab_strip(
        &self,
        active: usize,
        on_select: impl Fn(&usize, &mut Window, &mut App) + 'static,
        on_close: impl Fn(&usize, &mut Window, &mut App) + 'static,
        cx: &App,
    ) -> AnyElement {
        if self.shown.get() != Some(active) {
            self.scroll.scroll_to_item(active);
            self.shown.set(Some(active));
        }

        let on_select = std::rc::Rc::new(on_select);
        let on_close = std::rc::Rc::new(on_close);
        let close_variant = close_button_variant(cx);

        let tabs: Vec<Tab> = self
            .documents
            .iter()
            .enumerate()
            .map(|(ix, document)| {
                let group = SharedString::from(format!("document-tab-{ix}"));
                let select = on_select.clone();
                let close = on_close.clone();
                let title = document.read(cx).tab_title(cx);
                let close_id = SharedString::from(format!("document-close-{ix}"));

                // The button keeps its space while hidden -- `invisible` paints
                // nothing but still lays out -- so a tab does not change width
                // when the pointer enters it.
                let close_button = rsx! {
                    <div class="invisible" group_hover={(group.clone(), |this| this.visible())}>
                        <Button(close_id)
                            icon={gpui::component::IconName::Close}
                            xsmall
                            custom={close_variant}
                            tab_stop={false}
                            onClick={move |_, window, cx| {
                                // Closing must not also select the tab.
                                cx.stop_propagation();
                                close(&ix, window, cx);
                            }}
                        />
                    </div>
                };

                rsx! {
                    <Tab
                        base={Tab::new()}
                        label={title}
                        selected={ix == active}
                        group={group}
                        pl={px(16.)}
                        pr={px(1.)}
                        suffix={close_button}
                        on_click={move |_, window, cx| select(&ix, window, cx)}
                    />
                }
            })
            .collect();

        let empty_space = rsx! { <div class="h-full" flex_grow_1 min_w={px(8.)} /> };

        rsx! {
            <TabBar("document-tabs")
                underline
                small
                menu={true}
                track_scroll={&self.scroll}
                selected_index={active}
                children={tabs}
                last_empty_space={empty_space}
            />
        }
        .into_any_element()
    }
}

/// The colours a tab's close button wears.
fn close_button_variant(cx: &App) -> ButtonCustomVariant {
    let theme = cx.theme();
    ButtonCustomVariant::new(cx)
        .foreground(theme.secondary_foreground)
        .hover(*theme.tokens.secondary_hover)
        .active(*theme.tokens.secondary_active)
}

/// Which document the centre should show after the one at `removed` closed, out
/// of `remaining` left.
///
/// The tab that takes its place is the one that slid into its index; when the
/// last tab closed there is nothing left to show.
fn next_active(removed: usize, remaining: usize) -> Option<usize> {
    (remaining > 0).then(|| removed.min(remaining - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_the_active_tab_activates_the_one_that_took_its_place() {
        assert_eq!(next_active(0, 2), Some(0));
        assert_eq!(next_active(1, 2), Some(1));
    }

    #[test]
    fn closing_the_last_tab_activates_the_new_last_one() {
        // Three tabs, the third closed: index 2 no longer exists.
        assert_eq!(next_active(2, 2), Some(1));
        assert_eq!(next_active(5, 1), Some(0));
    }

    #[test]
    fn closing_the_only_tab_leaves_nothing_active() {
        assert_eq!(next_active(0, 0), None);
    }
}
