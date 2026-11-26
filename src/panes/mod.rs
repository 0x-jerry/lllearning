//! The side-pane seam: who may occupy a side slot, and how to build it.
//!
//! A side pane is a dockable panel plus the metadata the activity strip needs to
//! offer it. Everything the shell does with a pane goes through this module, so
//! adding a pane is a new file and one [`register_pane`] call — no edits to the
//! shell.
//!
//! # Writing a pane
//!
//! A pane is an ordinary GPUI entity with a [`Focusable`] handle and a `Render`
//! impl, plus the five dock traits every pane shares. Those are written by hand
//! rather than generated, because each pane's `title` and `dump` payload is its
//! own; [`register_pane`] supplies the dock's rebuild path, not the traits.
//! `panes/explorer.rs` is the shortest complete example.

pub mod browser;
pub mod explorer;
pub mod outline;

use std::{rc::Rc, sync::Arc};

use gpui::{App, Entity, Global, SharedString, WeakEntity, Window, component::dock::BasePanelView};
use lucide_icons::Icon;
use serde_json::Value;

use crate::{app::shell::AppShell, workspace::Workspace};

/// Which side of the shell a pane belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PaneSlot {
    Left,
    Right,
}

impl PaneSlot {
    /// The dock this slot lives in.
    pub fn placement(self) -> gpui::component::dock::DockPlacement {
        use gpui::component::dock::DockPlacement;
        match self {
            Self::Left => DockPlacement::Left,
            Self::Right => DockPlacement::Right,
        }
    }

    /// Both slots, in the order the shell lays them out.
    pub const ALL: [Self; 2] = [Self::Left, Self::Right];
}

/// What a provider is handed when its panel is built.
///
/// The dock's own builder context carries only the dock area and the persisted
/// panel state, so the workspace and the shell are handed over here instead.
pub struct PaneContext {
    pub workspace: Entity<Workspace>,
    pub shell: WeakEntity<AppShell>,
    /// The payload the panel wrote through its `dump`, or `None` for a fresh
    /// pane.
    pub saved: Option<Value>,
}

/// A kind of side pane.
///
/// One implementation per pane type. The registry keeps them; the shell reads
/// the list to draw the activity strip and to build the dock's initial layout.
pub trait PaneProvider: 'static {
    /// Stable id, written into the persisted layout as the panel name. Never
    /// rename one: a persisted layout referring to the old name comes back as an
    /// unknown panel.
    fn id(&self) -> &'static str;

    /// The name shown in the pane's tab and the strip's tooltip.
    fn title(&self) -> SharedString;

    /// The strip button's glyph.
    fn icon(&self) -> Icon;

    /// The side this pane belongs to.
    fn slot(&self) -> PaneSlot;

    /// Where in the strip it sits; lower first.
    fn order(&self) -> i32 {
        0
    }

    /// Build the panel, restoring `ctx.saved` if it is present.
    fn build(&self, ctx: PaneContext, window: &mut Window, cx: &mut App) -> Arc<dyn BasePanelView>;
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
///
/// Also registers the dock's panel builder, so `DockArea::load` can rebuild the
/// panel from a persisted layout without the shell knowing the pane exists.
pub fn register_pane(cx: &mut App, provider: Rc<dyn PaneProvider>) {
    if cx.try_global::<PaneRegistry>().is_none() {
        cx.set_global(PaneRegistry::default());
    }

    let id = provider.id();
    // The dock's registry is keyed by panel name and is global, so a rebuilt
    // panel finds the same provider it was written by.
    let for_builder = provider.clone();
    gpui::component::dock::register_panel(cx, id, move |context, window, cx| {
        let saved = match context.info() {
            gpui::component::dock::PanelInfo::Panel(value) => Some(value.clone()),
            _ => None,
        };
        let handles = ShellHandles::get(cx);
        for_builder.build(
            PaneContext {
                workspace: handles.workspace.clone(),
                shell: handles.shell.clone(),
                saved,
            },
            window,
            cx,
        )
    });

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

/// The handles a panel builder cannot get from its own context.
///
/// `PanelBuildContext` carries only the dock area and the persisted panel state,
/// but a rebuilt pane still needs the workspace and the shell. This global is
/// how they are reached; it is set once, when the shell is constructed.
#[derive(Clone)]
pub struct ShellHandles {
    pub workspace: Entity<Workspace>,
    pub shell: WeakEntity<AppShell>,
}

impl Global for ShellHandles {}

impl ShellHandles {
    /// Publish the handles for the dock's panel builders.
    pub fn set(cx: &mut App, workspace: Entity<Workspace>, shell: WeakEntity<AppShell>) {
        cx.set_global(Self { workspace, shell });
    }

    /// Read them back.
    ///
    /// A builder running without them is a wiring bug: the shell always
    /// publishes them before the dock area can load a layout.
    pub fn get(cx: &App) -> Self {
        cx.try_global::<Self>()
            .cloned()
            .expect("ShellHandles were published before the dock area was built")
    }
}

/// Register the panes this build ships.
pub fn register_builtins(cx: &mut App) {
    register_pane(cx, Rc::new(explorer::ExplorerPane));
    register_pane(cx, Rc::new(browser::BrowserPane));
    register_pane(cx, Rc::new(outline::OutlinePane));
}

/// Names a pane's dock traits need, re-exported so a pane module does not have
/// to import the dock crate directly.
#[doc(hidden)]
pub mod __private {}
