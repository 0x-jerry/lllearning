//! The vertical activity strip beside each side dock.
//!
//! One button per registered pane for the slot, plus a button that collapses the
//! dock. The strip is the only place the shell enumerates panes, and it reads
//! them from the registry — so a new pane appears here with no edit to this
//! file.

use gpui::component::{
    Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
};
use gpui::{
    App, InteractiveElement as _, IntoElement, ParentElement as _, SharedString, Styled as _,
    Window, div, prelude::FluentBuilder as _, px,
};
use gpui_rsx::rsx;

use crate::{
    icons,
    panes::{self, PaneSlot},
};

/// What a strip button reports when it is pressed: the provider id.
type SelectHandler = std::rc::Rc<dyn Fn(&SharedString, &mut Window, &mut App) + 'static>;

/// What the collapse button reports when it is pressed: which side.
type ToggleHandler = std::rc::Rc<dyn Fn(&PaneSlot, &mut Window, &mut App) + 'static>;

/// The strip beside one dock.
pub struct ActivityBar {
    slot: PaneSlot,
    /// The panes this strip offers, resolved from the registry by the caller.
    providers: Vec<std::rc::Rc<dyn panes::PaneProvider>>,
    /// The provider id the dock is showing, so its button reads as selected.
    active: Option<String>,
    /// Whether the dock is open, so the collapse button can point the right way.
    open: bool,
    /// `Rc` because every button shares one handler.
    on_select: Option<SelectHandler>,
    on_toggle: Option<ToggleHandler>,
}

impl ActivityBar {
    /// A strip for `slot`, with `active` marking the pane on show.
    ///
    /// The providers are resolved by the caller, which already holds an
    /// immutable borrow of the app while it builds the listeners.
    pub fn new(
        slot: PaneSlot,
        active: Option<String>,
        providers: Vec<std::rc::Rc<dyn panes::PaneProvider>>,
    ) -> Self {
        Self {
            slot,
            providers,
            active,
            open: true,
            on_select: None,
            on_toggle: None,
        }
    }

    /// Called with the provider id when a pane's button is pressed.
    pub fn on_select(
        mut self,
        handler: impl Fn(&SharedString, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_select = Some(std::rc::Rc::new(handler));
        self
    }

    /// Called with the slot when the collapse button is pressed.
    pub fn on_toggle(
        mut self,
        handler: impl Fn(&PaneSlot, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_toggle = Some(std::rc::Rc::new(handler));
        self
    }

    /// Whether the dock this strip belongs to is open, so the collapse button
    /// can show what pressing it will do.
    pub fn set_open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// The strip as an element.
    ///
    /// The theme values are passed in rather than read here, because the caller
    /// already holds an immutable borrow of the app while it builds the
    /// listeners.
    pub fn render(self, border: gpui::Hsla, sidebar: gpui::Hsla) -> gpui::AnyElement {
        let slot = self.slot;
        let providers = self.providers;
        let active = self.active.clone();
        let on_select = self.on_select;
        let on_toggle = self.on_toggle;

        let buttons: Vec<gpui::AnyElement> = providers
            .into_iter()
            .map(|provider| {
                let id = provider.id();
                let title = provider.title();
                let is_active = active.as_deref().is_some_and(|current| current == id);
                let glyph = provider.icon();
                let id_for_click = SharedString::from(id.to_owned());
                // The handler is shared between every button, so it is cloned
                // into each closure rather than moved out of the strip.
                let on_select = on_select.clone();

                Button::new(SharedString::from(format!(
                    "activity-{}-{id}",
                    slot_name(slot)
                )))
                // The lucide font is not an SVG asset, so the glyph goes in as a
                // child element rather than through `Button::icon`.
                .child(icons::icon(glyph, 16.))
                .ghost()
                .small()
                .tooltip(title)
                .selected(is_active)
                .on_click(move |_, window, cx| {
                    if let Some(handler) = &on_select {
                        handler(&id_for_click, window, cx);
                    }
                })
                .into_any_element()
            })
            .collect();

        // The button points the way it will move the dock: an open dock shows
        // the "close" glyph, a closed one the "open" glyph.
        let collapse = match (slot, self.open) {
            (PaneSlot::Left, true) => lucide_icons::Icon::PanelLeftClose,
            (PaneSlot::Left, false) => lucide_icons::Icon::PanelLeftOpen,
            (PaneSlot::Right, true) => lucide_icons::Icon::PanelRightClose,
            (PaneSlot::Right, false) => lucide_icons::Icon::PanelRightOpen,
        };
        let toggle_slot = slot;

        rsx! {
            <div
                id={SharedString::from(format!("activity-bar-{}", slot_name(slot)))}
                class="flex flex-col items-center gap-1 py-1"
                w={px(36.)}
                h_full
                flex_none
                bg={sidebar}
                border_color={border}
                when={(slot == PaneSlot::Left, |el| el.border_r_1())}
                when={(slot == PaneSlot::Right, |el| el.border_l_1())}
            >
                {...buttons.into_iter()}
                <Button("activity-toggle")
                    ghost
                    small
                    tooltip={"Show or hide this panel"}
                    onClick={move |_, window, cx| {
                        if let Some(handler) = &on_toggle {
                            handler(&toggle_slot, window, cx);
                        }
                    }}
                >
                    {icons::icon(collapse, 16.)}
                </Button>
            </div>
        }
        .into_any_element()
    }
}

/// The name of a slot, for element ids.
fn slot_name(slot: PaneSlot) -> &'static str {
    match slot {
        PaneSlot::Left => "left",
        PaneSlot::Right => "right",
    }
}
