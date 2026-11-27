//! The side-pane seam: who may occupy a side slot, and how to build it.
//!
//! A pane is an ordinary GPUI entity with a [`Focusable`] handle and a `Render`
//! impl, handed back from [`PaneProvider::build`] as a [`PaneContent`]. A pane
//! whose contents live outside the element tree supplies
//! [`PaneContent::activation`]; a pane that remembers something supplies a
//! [`PaneController`]. Adding one is a new file plus a [`register_pane`] call.

pub mod browser;
pub mod explorer;
pub mod outline;

use std::rc::Rc;

use gpui::{AnyView, App, Global, SharedString, Window};
use lucide_icons::Icon;

use crate::workspace::Workspace;

/// Which side of the shell a pane belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PaneSlot {
    Left,
    Right,
}

impl PaneSlot {
    /// Both slots, in the order the shell lays them out.
    pub const ALL: [Self; 2] = [Self::Left, Self::Right];
}

/// What a provider is handed when its panel is built.
pub struct PaneContext {
    pub workspace: gpui::Entity<Workspace>,
    pub shell: gpui::WeakEntity<crate::app::shell::AppShell>,
}

/// What a pane remembers between runs, opaque to the dock. A value this build
/// cannot decode leaves the pane at its defaults.
pub type ViewState = serde_json::Value;

/// Object-safe handle over a side pane's saved state. Only the provider knows
/// the concrete type, so only the provider can hand one out. Both methods have
/// defaults, because most panes keep nothing.
pub trait PaneController {
    /// The state the pane wants back next run, or `None`.
    fn view_state(&self, cx: &App) -> Option<ViewState> {
        let _ = cx;
        None
    }

    /// Apply what [`Self::view_state`] returned on the previous run, once, after
    /// the pane is built.
    fn restore_view_state(&self, state: &ViewState, window: &mut Window, cx: &mut App) {
        let _ = (state, window, cx);
    }
}

/// The controller for a pane that keeps nothing.
pub struct StatelessPane;

impl PaneController for StatelessPane {}

/// Called with `true` when the pane is placed on screen and `false` when it
/// leaves. For panes that own something the element tree does not, such as the
/// browser's native child window.
pub type Activation = Rc<dyn Fn(bool, &mut Window, &mut App)>;

/// What a provider returns: the view, its controller, and how to tell it it left
/// the screen.
pub struct PaneContent {
    pub view: AnyView,
    pub controller: Rc<dyn PaneController>,
    pub activation: Option<Activation>,
}

impl PaneContent {
    /// A pane with nothing to do when it is hidden and nothing to remember.
    pub fn new(view: impl Into<AnyView>) -> Self {
        Self {
            view: view.into(),
            controller: Rc::new(StatelessPane),
            activation: None,
        }
    }
}

/// A kind of side pane.
pub trait PaneProvider: 'static {
    /// Stable id, written into the session file; never rename one.
    fn id(&self) -> &'static str;

    /// The name shown in the strip's tooltip.
    fn title(&self) -> SharedString;

    /// The strip button's glyph.
    fn icon(&self) -> Icon;

    /// The side this pane belongs to.
    fn slot(&self) -> PaneSlot;

    /// Where in the strip it sits; lower first.
    fn order(&self) -> i32 {
        0
    }

    /// Build the pane's view. Runs on the render thread, so anything expensive
    /// must be started and awaited elsewhere.
    fn build(&self, ctx: PaneContext, window: &mut Window, cx: &mut App) -> PaneContent;
}

/// Every registered pane.
#[derive(Default)]
pub struct PaneRegistry {
    providers: Vec<Rc<dyn PaneProvider>>,
}

impl Global for PaneRegistry {}

impl PaneRegistry {
    /// The providers for `slot`, in strip order.
    pub fn for_slot(&self, slot: PaneSlot) -> Vec<Rc<dyn PaneProvider>> {
        let mut found: Vec<Rc<dyn PaneProvider>> = self
            .providers
            .iter()
            .filter(|provider| provider.slot() == slot)
            .cloned()
            .collect();
        found.sort_by_key(|provider| provider.order());
        found
    }

    /// A provider by id.
    pub fn by_id(&self, id: &str) -> Option<Rc<dyn PaneProvider>> {
        self.providers
            .iter()
            .find(|provider| provider.id() == id)
            .cloned()
    }
}

/// Add a pane type.
pub fn register_pane(cx: &mut App, provider: Rc<dyn PaneProvider>) {
    if cx.try_global::<PaneRegistry>().is_none() {
        cx.set_global(PaneRegistry::default());
    }
    cx.global_mut::<PaneRegistry>().providers.push(provider);
}

/// The providers for `slot`, in strip order.
pub fn panes_for(cx: &App, slot: PaneSlot) -> Vec<Rc<dyn PaneProvider>> {
    cx.try_global::<PaneRegistry>()
        .map(|registry| registry.for_slot(slot))
        .unwrap_or_default()
}

/// A provider by id.
pub fn pane_by_id(cx: &App, id: &str) -> Option<Rc<dyn PaneProvider>> {
    cx.try_global::<PaneRegistry>()
        .and_then(|registry| registry.by_id(id))
}

/// Register the panes this build ships.
pub fn register_builtins(cx: &mut App) {
    register_pane(cx, Rc::new(explorer::ExplorerPane));
    register_pane(cx, Rc::new(browser::BrowserPane));
    register_pane(cx, Rc::new(outline::OutlinePane));
}
