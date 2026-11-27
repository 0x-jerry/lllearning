//! The shell, its chrome, and the built-in registrations.

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

/// The one place a feature is switched on; called once from `main`.
pub fn init(cx: &mut App) {
    if cx.try_global::<PaneRegistry>().is_none() {
        cx.set_global(PaneRegistry::default());
    }
    if cx.try_global::<DocumentRegistry>().is_none() {
        cx.set_global(DocumentRegistry::default());
    }

    panes::register_builtins(cx);
    documents::register_builtins(cx);
    documents::text::bind_keys(cx);
    popup::dialog::init(cx);
}
