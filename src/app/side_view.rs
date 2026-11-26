//! One side dock: the activity strip beside it, and the pane it is showing.
//!
//! The strip and the content are one thing because they answer one question —
//! which pane is in this dock, and is the dock open. Splitting them would mean
//! two copies of that answer.
//!
//! Panes are built once and kept. Rebuilding on every switch would recreate the
//! browser's WebView2 — slow, and it re-arms the profile lock (risk R2) — and
//! would throw away the explorer's expansion state.

use std::collections::HashMap;

use gpui::component::ActiveTheme as _;
use gpui::{AnyElement, AnyView, App, Context, IntoElement, SharedString, Window, px};

use crate::{
    app::{
        activity_bar::ActivityBar,
        shell::{AppShell, empty_state, frame},
    },
    panes::{self, Activation, PaneContext, PaneSlot},
    state::SideState,
    workspace::Workspace,
};

/// A pane the dock has built, whether or not it is the one on show.
struct MountedPane {
    view: AnyView,
    /// What to call when the pane leaves or re-enters the screen. The browser
    /// uses it to hide its page (risk R1b).
    activation: Option<Activation>,
    /// Whether the pane is the one the dock is showing.
    active: bool,
}

impl MountedPane {
    fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut App) {
        if self.active == active {
            return;
        }
        self.active = active;
        if let Some(activation) = &self.activation {
            activation(active, window, cx);
        }
    }
}

/// One side dock.
pub struct SideView {
    slot: PaneSlot,
    /// The provider id the dock is showing.
    active: Option<String>,
    open: bool,
    /// Every pane built for this dock, by provider id.
    panes: HashMap<String, MountedPane>,
    /// The width the session asked for, used until the resizable group has
    /// measured itself.
    saved_width: Option<gpui::Pixels>,
    workspace: gpui::Entity<Workspace>,
    shell: gpui::WeakEntity<AppShell>,
}

impl SideView {
    /// A dock for `slot`, showing nothing until [`Self::apply`] or
    /// [`Self::show`] says otherwise.
    pub fn new(
        slot: PaneSlot,
        workspace: gpui::Entity<Workspace>,
        shell: gpui::WeakEntity<AppShell>,
    ) -> Self {
        Self {
            slot,
            active: None,
            open: false,
            panes: HashMap::new(),
            saved_width: None,
            workspace,
            shell,
        }
    }

    /// Whether the dock is open. A closed dock draws no content, and its strip
    /// marks no pane as selected.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The provider id on show, open or not.
    pub fn active(&self) -> Option<&str> {
        self.active.as_deref()
    }

    /// The width the session asked for, if it asked for one.
    pub fn saved_width(&self) -> Option<gpui::Pixels> {
        self.saved_width
    }

    /// Adopt one dock's saved state, falling back to the slot's first provider
    /// when the saved id names a pane this build does not have.
    pub fn apply(&mut self, saved: &SideState, cx: &App) {
        let known = saved
            .active
            .as_deref()
            .and_then(|id| panes::pane_by_id(cx, id));
        let id = known.map(|provider| provider.id().to_owned()).or_else(|| {
            panes::panes_for(cx, self.slot)
                .first()
                .map(|provider| provider.id().to_owned())
        });

        self.open = saved.open && id.is_some();
        self.active = id;
        if let Some(width) = saved.width.filter(|width| *width > 0.) {
            self.saved_width = Some(px(width));
        }
    }

    /// Show `provider_id`, or collapse the dock if it is already showing it.
    ///
    /// Returns whether anything changed, so the caller knows whether the
    /// session is worth rewriting.
    pub fn show(&mut self, provider_id: &str, window: &mut Window, cx: &mut App) -> bool {
        // Pressing the button of the pane on show means "put that away", which
        // is the only collapse affordance the strip has.
        if self.open && self.active() == Some(provider_id) {
            self.set_open(false, window, cx);
            return true;
        }
        if panes::pane_by_id(cx, provider_id).is_none() {
            return false;
        }

        self.active = Some(provider_id.to_owned());
        self.open = true;
        self.ensure(provider_id, window, cx);
        self.activate(window, cx);
        true
    }

    /// Open or collapse the dock, keeping the pane it shows.
    pub fn toggle(&mut self, window: &mut Window, cx: &mut App) {
        self.set_open(!self.open, window, cx);
    }

    fn set_open(&mut self, open: bool, window: &mut Window, cx: &mut App) {
        self.open = open;
        self.activate(window, cx);
    }

    /// Build the pane on show if it is not built already, so a closed dock can
    /// be reopened and a restored one has something to draw.
    pub fn ensure_active(&mut self, window: &mut Window, cx: &mut App) {
        if let Some(id) = self.active.clone() {
            self.ensure(&id, window, cx);
            self.activate(window, cx);
        }
    }

    fn ensure(&mut self, provider_id: &str, window: &mut Window, cx: &mut App) {
        if self.panes.contains_key(provider_id) {
            return;
        }
        let Some(provider) = panes::pane_by_id(cx, provider_id) else {
            return;
        };

        let content = provider.build(
            PaneContext {
                workspace: self.workspace.clone(),
                shell: self.shell.clone(),
            },
            window,
            cx,
        );
        // Built off screen, so a pane whose contents can leave the screen starts
        // hidden and the next transition shows it.
        self.panes.insert(
            provider_id.to_owned(),
            MountedPane {
                view: content.view,
                activation: content.activation,
                active: false,
            },
        );
    }

    /// Tell every pane that changed whether it is the one on screen.
    ///
    /// Only transitions are reported, never every frame, so a pane's own render
    /// stays the authority on what it draws.
    fn activate(&mut self, window: &mut Window, cx: &mut App) {
        let active = self.open.then(|| self.active.clone()).flatten();
        for (id, pane) in self.panes.iter_mut() {
            pane.set_active(active.as_deref() == Some(id.as_str()), window, cx);
        }
    }

    /// This dock's state, for the session file.
    ///
    /// `width` is what the resizable group measured. A closed dock measures
    /// zero, so the width the user last chose is kept from `saved_width` rather
    /// than being forgotten on every save.
    pub fn state(&self, width: Option<f32>) -> SideState {
        SideState {
            active: self.active.clone(),
            open: self.open,
            width: width.or_else(|| self.saved_width.map(|width| width.as_f32())),
        }
    }

    /// The strip beside the dock.
    pub fn render_strip(&self, cx: &mut Context<AppShell>) -> AnyElement {
        // A collapsed dock shows no pane, so its strip must not mark one as
        // selected -- otherwise the button would read as active while the pane
        // it names is hidden.
        let shown = self.open.then(|| self.active.clone()).flatten();
        let providers = panes::panes_for(cx, self.slot);
        let border = cx.theme().border;
        let sidebar = cx.theme().tokens.sidebar.color;
        let slot = self.slot;

        ActivityBar::new(slot, shown, providers)
            .on_select(
                cx.listener(move |this: &mut AppShell, id: &SharedString, window, cx| {
                    this.show_pane(slot, id, window, cx);
                }),
            )
            .render(border, sidebar)
    }

    /// The dock's content: the pane on show, framed.
    pub fn render_content(&self, cx: &App) -> AnyElement {
        let body = self
            .active
            .as_ref()
            .and_then(|id| self.panes.get(id))
            .map(|pane| pane.view.clone().into_any_element())
            .unwrap_or_else(|| empty_state("No pane", "Pick one in the strip.", cx));

        let id = match self.slot {
            PaneSlot::Left => "pane-left",
            PaneSlot::Right => "pane-right",
        };
        frame(id, None, body, cx)
    }
}
