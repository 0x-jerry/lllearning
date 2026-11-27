//! Application layer: the shell, its chrome, and the built-in registrations.
//!
//! [`init`] is the single place a feature is switched on. Adding a pane or a
//! document viewer is a line here, which is what "extensible without touching
//! the shell" means in practice.

pub mod activity_bar;
pub mod documents_view;
pub mod popup;
pub mod shell;
pub mod side_view;
pub mod title_bar;

use gpui::App;

use crate::{
    documents::{self, DocumentRegistry},
    panes::{self, PaneRegistry},
};

/// Register the skin's fonts, the built-in panes and the built-in viewers.
///
/// Called once from `main`, after `gpui::init`. Everything here is additive: a
/// later call adds providers rather than replacing them.
pub fn init(cx: &mut App) {
    // The registries must exist before anything asks for their contents.
    if cx.try_global::<PaneRegistry>().is_none() {
        cx.set_global(PaneRegistry::default());
    }
    if cx.try_global::<DocumentRegistry>().is_none() {
        cx.set_global(DocumentRegistry::default());
    }

    panes::register_builtins(cx);
    documents::register_builtins(cx);
    // `ctrl-s` / `cmd-s`, scoped to a document viewer so no other input in the
    // window can claim it.
    documents::text::bind_keys(cx);
    // Escape and Enter inside a prompt, and Escape inside a context menu.
    popup::dialog::init(cx);
}
