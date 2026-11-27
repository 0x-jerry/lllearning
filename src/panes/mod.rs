//! The side-pane seam: who may occupy a side slot, and how to build it.
//!
//! A side pane is a view plus the metadata the activity strip needs to offer it.
//! Everything the shell does with a pane goes through this module, so adding a
//! pane is a new file and one [`register_pane`] call — no edits to the shell.
//!
//! # Writing a pane
//!
//! A pane is an ordinary GPUI entity with a [`Focusable`] handle and a `Render`
//! impl. That is the whole contract: the shell frames the built view in one of
//! its docks, so nothing here knows how the pane is placed. `panes/explorer.rs`
//! is the shortest complete example.
//!
//! A pane whose contents can leave the screen — the browser, whose page is a
//! native child window that outlives the element placing it (risk R1b) — also
//! supplies [`PaneContent::activation`], and the shell calls it whenever the
//! pane is shown or hidden.
//!
//! A pane that remembers something between runs supplies a [`PaneController`]
//! with it, and the dock asks it for that state when the session is written and
//! hands it back when the pane is built.

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
    /// The folder tree, for a pane that shows or changes it.
    pub workspace: gpui::Entity<Workspace>,
    /// The shell, for a pane that drives the rest of the app.
    pub shell: gpui::WeakEntity<crate::app::shell::AppShell>,
}

/// What a pane wants to remember about itself between runs.
///
/// The same bargain the document viewers strike (`documents::ViewState`): the
/// pane defines the shape, the dock moves the value into the session file and
/// back without looking inside, and a value this build cannot decode leaves the
/// pane at its defaults rather than failing to build it (risk R12).
pub type ViewState = serde_json::Value;

/// Object-safe handle over a side pane, so the dock can ask any of them what it
/// wants kept without knowing the pane type.
///
/// A built pane is an [`AnyView`], which carries an entity id and nothing else;
/// only the provider knows the concrete type, so only the provider can hand out
/// this. Every method has a default, because most panes keep nothing.
pub trait PaneController {
    /// The state the pane wants back next run, or `None` when its view is all
    /// it needs.
    fn view_state(&self, cx: &App) -> Option<ViewState> {
        let _ = cx;
        None
    }

    /// Apply what [`Self::view_state`] returned on the previous run.
    ///
    /// Called once, after the pane is built. A value this build does not
    /// understand leaves the pane at its defaults rather than failing to build
    /// it (risk R12).
    fn restore_view_state(&self, state: &ViewState, window: &mut Window, cx: &mut App) {
        let _ = (state, window, cx);
    }
}

/// The controller for a pane that keeps nothing.
pub struct StatelessPane;

impl PaneController for StatelessPane {}

/// Called with `true` when the pane is placed on screen and `false` when it
/// leaves.
///
/// A pane that owns something the element tree does not — the browser's native
/// child window — is what this exists for; every other pane ignores it.
pub type Activation = Rc<dyn Fn(bool, &mut Window, &mut App)>;

/// What a provider returns: the view, its controller, and how to tell it it
/// left the screen.
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
///
/// One implementation per pane type. The registry keeps them; the shell reads
/// the list to draw the activity strip and to build a dock's pane.
pub trait PaneProvider: 'static {
    /// Stable id, written into the session file. Never rename one: a session
    /// naming the old id comes back with no pane in that dock.
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

    /// Build the pane's view.
    ///
    /// Runs on the render thread, so a provider that starts something expensive
    /// must start it and return, not wait for it.
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
