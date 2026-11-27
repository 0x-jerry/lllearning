//! One side dock: the activity strip beside it and the pane it shows.
//! Panes are built once and kept, so switching does not restart a WebView2 or
//! lose the explorer's expansion state.

use std::collections::HashMap;
use std::rc::Rc;

use gpui::component::ActiveTheme as _;
use gpui::{
    AnyElement, AnyView, App, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Window, div, px,
};
use gpui_rsx::rsx;

use crate::{
    app::{activity_bar::ActivityBar, shell::empty_state},
    panes::{self, Activation, PaneContext, PaneController, PaneSlot, ViewState},
    state::SideState,
    workspace::Workspace,
};

/// A pane the dock has built, whether or not it is on show.
struct MountedPane {
    view: AnyView,
    controller: Rc<dyn PaneController>,
    /// What to call when the pane leaves or re-enters the screen; the browser
    /// uses it to hide its page.
    activation: Option<Activation>,
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
    panes: HashMap<String, MountedPane>,
    /// The state the session saved, with the pane id it belongs to. Kept here
    /// because the pane is built later, or never while the dock stays closed.
    saved_state: Option<(String, ViewState)>,
    workspace: Entity<Workspace>,
    shell: gpui::WeakEntity<crate::app::shell::AppShell>,
}

impl SideView {
    /// A dock that shows nothing until [`Self::apply`] or [`Self::show`] says
    /// otherwise.
    pub fn new(
        slot: PaneSlot,
        workspace: Entity<Workspace>,
        shell: gpui::WeakEntity<crate::app::shell::AppShell>,
    ) -> Self {
        Self {
            slot,
            active: None,
            open: false,
            panes: HashMap::new(),
            saved_state: None,
            workspace,
            shell,
        }
    }

    /// Whether the dock is open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The provider id on show, open or not.
    pub fn active(&self) -> Option<&str> {
        self.active.as_deref()
    }

    /// Adopt one dock's saved state, falling back to the slot's first provider
    /// when the saved id names a pane this build does not have.
    pub fn apply(&mut self, saved: &SideState, cx: &mut Context<Self>) {
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
        // Only for the pane that wrote it: another pane would restore the wrong thing.
        self.saved_state = match (&id, saved.active.as_deref()) {
            (Some(id), Some(saved_id)) if id == saved_id => {
                saved.state.clone().map(|state| (id.clone(), state))
            }
            _ => None,
        };
        self.active = id;
        cx.notify();
    }

    /// Show `provider_id`, or collapse the dock if it already shows it. Returns
    /// whether anything changed.
    pub fn show(&mut self, provider_id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
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
        cx.notify();
        true
    }

    /// Open or collapse the dock, keeping the pane it shows.
    pub fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_open(!self.open, window, cx);
    }

    fn set_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.open = open;
        if open {
            self.ensure_active(window, cx);
        }
        self.activate(window, cx);
        cx.notify();
    }

    /// Build the pane on show if it is not built already.
    ///
    /// A collapsed dock builds nothing: the browser's WebView2 starts a browser
    /// process.
    pub fn ensure_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        if let Some(id) = self.active.clone() {
            self.ensure(&id, window, cx);
            self.activate(window, cx);
        }
    }

    fn ensure(&mut self, provider_id: &str, window: &mut Window, cx: &mut Context<Self>) {
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
        // What the session saved for this pane, if it was this pane that saved it.
        if let Some((saved_id, state)) = &self.saved_state
            && saved_id == provider_id
        {
            content.controller.restore_view_state(state, window, cx);
        }
        // Built off screen, so it starts hidden and the next transition shows it.
        self.panes.insert(
            provider_id.to_owned(),
            MountedPane {
                view: content.view,
                controller: content.controller,
                activation: content.activation,
                active: false,
            },
        );
        cx.notify();
    }

    /// Tell every pane that changed whether it is the one on screen. Only
    /// transitions are reported, never every frame.
    fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let active = self.open.then(|| self.active.clone()).flatten();
        for (id, pane) in self.panes.iter_mut() {
            pane.set_active(active.as_deref() == Some(id.as_str()), window, cx);
        }
    }

    /// This dock's state, for the session file.
    ///
    /// `width` comes from the shell: the group measures the dock, and a hidden
    /// panel measures nothing.
    pub fn state(&self, width: Option<f32>, cx: &App) -> SideState {
        SideState {
            active: self.active.clone(),
            open: self.open,
            width,
            state: self.pane_state(cx),
        }
    }

    /// The state of the pane on show. A pane the dock never built has no view
    /// to ask, so the state it came in with is the answer.
    fn pane_state(&self, cx: &App) -> Option<ViewState> {
        let active = self.active.as_ref()?;
        match self.panes.get(active) {
            Some(pane) => pane.controller.view_state(cx),
            None => self
                .saved_state
                .as_ref()
                .filter(|(saved_id, _)| saved_id == active)
                .map(|(_, state)| state.clone()),
        }
    }

    /// The strip that toggles this dock and switches its pane.
    fn strip(&self, cx: &mut App) -> AnyElement {
        // A collapsed dock shows no pane, so its strip marks none as selected.
        let shown = self.open.then(|| self.active.clone()).flatten();
        let providers = panes::panes_for(cx, self.slot);
        let border = cx.theme().border;
        let sidebar = cx.theme().tokens.sidebar.color;
        let slot = self.slot;
        let shell = self.shell.clone();

        ActivityBar::new(slot, shown, providers)
            .on_select(move |id, window, cx| {
                let id = id.clone();
                let _ = shell.update(cx, |shell, cx| shell.show_pane(slot, &id, window, cx));
            })
            .render(border, sidebar)
    }
}

impl Render for SideView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The shell places a collapsed dock's strip in the centre row instead,
        // so the button that reopens it survives its panel being hidden.
        let strip = self.strip(cx);
        if !self.open {
            return strip;
        }

        let background = cx.theme().tokens.background;
        let frame_id = match self.slot {
            PaneSlot::Left => "pane-left",
            PaneSlot::Right => "pane-right",
        };
        let body = match self.active.as_ref().and_then(|id| self.panes.get(id)) {
            Some(pane) => pane.view.clone().into_any_element(),
            None => empty_state("No pane", "Pick one in the strip.", cx),
        };

        // Left bar, content, right bar.
        let mut strip = Some(strip);
        let mut row: Vec<AnyElement> = Vec::with_capacity(2);
        if self.slot == PaneSlot::Left {
            row.extend(strip.take());
        }
        row.push(
            rsx! {
                <div
                    id={SharedString::from(frame_id)}
                    class="flex flex-col flex-1 min-w-0 min-h-0"
                    bg={background}
                >
                    <div class="flex-1 min-h-0 overflow-hidden">{body}</div>
                </div>
            }
            .into_any_element(),
        );
        if self.slot == PaneSlot::Right {
            row.extend(strip.take());
        }

        rsx! {
            <div class="flex flex-row size-full min-h-0 items-stretch">
                {...row.into_iter()}
            </div>
        }
        .into_any_element()
    }
}
